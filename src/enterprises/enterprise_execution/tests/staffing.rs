use super::*;

#[test]
fn autonomous_expansion_reconsiders_smaller_racket_after_subordinate_consumes_command_capacity() {
    let registry = build_registry();
    let profile = NeighborhoodProfile {
        economy: NeighborhoodEconomyProfile {
            wealth: rating(10),
            commercial_activity: rating(100),
            illicit_demand: rating(10),
        },
        institutions: NeighborhoodInstitutionProfile {
            police_presence: rating(0),
        },
    };
    let mut fixture = make_test_enterprise_fixture_with_inputs(
        OrganizationKind::Criminal,
        AutonomyLevel::Delegated,
        profile,
        Some(rating(15)),
    );
    let executive = fixture.authority.manager;
    let EnterpriseLocation::Neighborhood(home) = fixture.location else {
        unreachable!();
    };
    let other = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Commercial branch".into(),
            profile,
        },
    )
    .unwrap();
    let captain = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Experienced branch captain".into(),
            organization: Some(fixture.organization),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager: captain,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(other)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let front = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Cash counter".into(),
            kind: BusinessKind::Pawnshop,
            functions: BTreeSet::from([BusinessFunction::CashIntensive]),
            neighborhood: home,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .unwrap();
    fund_enterprise_fixture_cash(&mut fixture, 200_000);
    fixture.state.advance_clock(SimDuration::from_minutes(1439));
    let mut replay =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();

    let tick = run_tick(&registry, &mut fixture.state);
    assert_eq!(tick, run_tick(&registry, &mut replay));
    let established: Vec<_> = fixture
        .state
        .enterprises()
        .active_for_organization(fixture.organization)
        .map(|enterprise| {
            (
                enterprise.manager(),
                enterprise.kind(),
                enterprise.location(),
            )
        })
        .collect();
    assert_eq!(
        established,
        vec![
            (
                captain,
                EnterpriseKind::Protection,
                EnterpriseLocation::Neighborhood(other)
            ),
            (
                executive,
                EnterpriseKind::LoanSharking,
                EnterpriseLocation::Business(front)
            ),
        ]
    );
    let workload = manager_workload(&fixture.state, executive).unwrap();
    assert_eq!((workload.required, workload.available), (5, 5));
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&replay).unwrap()
    );
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn working_capital_reserves_the_actual_active_workforce_wages() {
    use crate::enterprises::autonomous_planning::resolve_committed_working_capital;

    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let baseline = resolve_committed_working_capital(&registry, &fixture.state, &BTreeMap::new())
        .unwrap()[&fixture.cash];
    validate_staff_enterprise(&fixture.state, enterprise, 9)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let staffed = resolve_committed_working_capital(&registry, &fixture.state, &BTreeMap::new())
        .unwrap()[&fixture.cash];
    assert_eq!(
        staffed,
        baseline.checked_add(Money::from_cents(5 * 1_500)).unwrap()
    );
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
    let location = fixture.location;
    let initial_load = manager_workload(&fixture.state, manager).unwrap();
    assert_eq!(initial_load.rackets, 1);
    assert_eq!(initial_load.associates, 4);
    assert_eq!(initial_load.required, 5);
    assert_eq!(initial_load.available, 27);
    assert!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 20,
                supporting_businesses: 0,
            },
            Some(enterprise)
        )
        .is_ok()
    );
    assert!(matches!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 25,
                supporting_businesses: 0,
            },
            Some(enterprise)
        ),
        Ok(())
    ));
    validate_staff_enterprise(&fixture.state, enterprise, 12)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(matches!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 14,
                supporting_businesses: 0,
            },
            None
        ),
        Err(EnterpriseError::ManagerWorkloadExceeded {
            required: 28,
            available: 27,
            ..
        })
    ));
    assert!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 13,
                supporting_businesses: 0,
            },
            None
        )
        .is_ok()
    );
    // Idle hires are not phantom management bandwidth. Only reassigning
    // operational responsibility to a capable captain increases real coverage.
    let additional_captain = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Additional Captain".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(manager),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(90).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(matches!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 14,
                supporting_businesses: 0,
            },
            None
        ),
        Err(EnterpriseError::ManagerWorkloadExceeded { .. })
    ));
    assert!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            additional_captain,
            ManagedJob {
                kind: EnterpriseKind::Protection,
                location,
                associates: 14,
                supporting_businesses: 0,
            },
            None
        )
        .is_ok()
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
fn an_unskilled_captain_cannot_grow_their_capacity_by_hiring_more_workers() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture_with_inputs(
        OrganizationKind::Criminal,
        AutonomyLevel::Delegated,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(60),
                commercial_activity: rating(70),
                illicit_demand: rating(50),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(40),
            },
        },
        None,
    );
    let racket = establish_protection(&registry, &mut fixture);
    let boss = fixture.authority.manager;
    let baseline = manager_workload(&fixture.state, boss).unwrap();
    assert_eq!((baseline.required, baseline.available), (5, 5));
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_staff_enterprise(&fixture.state, racket, 5).err(),
        Some(EnterpriseError::ManagerWorkloadExceeded {
            manager: boss,
            required: 6,
            available: 5,
        })
    );
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    // Recruiting an idle body still does nothing; actual reassignment to a
    // skilled captain is what makes extra field workers manageable.
    let captain = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Qualified district captain".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(boss),
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
    assert_eq!(
        validate_staff_enterprise(&fixture.state, racket, 5).err(),
        Some(EnterpriseError::ManagerWorkloadExceeded {
            manager: boss,
            required: 6,
            available: 5,
        })
    );
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager: captain,
            scopes: BTreeSet::from([fixture.authority.scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    validate_reassign_enterprise(
        &fixture.state,
        racket,
        MandateAuthority {
            mandate,
            manager: captain,
            scope: fixture.authority.scope,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    validate_staff_enterprise(&fixture.state, racket, 9)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let boss_load = manager_workload(&fixture.state, boss).unwrap();
    assert_eq!(
        (
            boss_load.command_lines,
            boss_load.required,
            boss_load.available
        ),
        (1, 1, 1)
    );
    let captain_load = manager_workload(&fixture.state, captain).unwrap();
    assert_eq!(
        (
            captain_load.associates,
            captain_load.required,
            captain_load.available
        ),
        (9, 10, 27)
    );
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(manager_workload(&restored, captain), Some(captain_load));
    validate_state_against_registry(&registry, &restored).unwrap();

    // Tightening control of the captain overloads the executive without
    // changing the captain's own work. Leadership must still be able to
    // release workers and reduce daily costs while resolving the command issue.
    crate::world::world_system::validate_set_character_autonomy(
        &fixture.state,
        fixture.organization,
        captain,
        AutonomyLevel::Guided,
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let overloaded = manager_workload(&fixture.state, boss).unwrap();
    assert_eq!((overloaded.required, overloaded.available), (2, 1));
    let mut replay =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut replay] {
        let before = bincode::serialize(state).unwrap();
        assert_eq!(
            validate_staff_enterprise(state, racket, 10).err(),
            Some(EnterpriseError::ManagerWorkloadExceeded {
                manager: boss,
                required: 2,
                available: 1,
            })
        );
        assert_eq!(bincode::serialize(state).unwrap(), before);
        validate_staff_enterprise(state, racket, 8)
            .expect("releasing workers must not require spare command capacity")
            .commit(state)
            .unwrap();
        assert_eq!(manager_workload(state, boss), Some(overloaded));
        assert_eq!(manager_workload(state, captain).unwrap().required, 9);
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&replay).unwrap()
    );
}

#[test]
fn executive_command_is_a_real_bottleneck_until_an_underboss_delegates_the_crews() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture_with_inputs(
        OrganizationKind::Criminal,
        AutonomyLevel::Delegated,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(60),
                commercial_activity: rating(70),
                illicit_demand: rating(50),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(40),
            },
        },
        None,
    );
    let executive = fixture.authority.manager;
    let first_district = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => unreachable!(),
    };
    let profile = fixture
        .state
        .world()
        .get_neighborhood(first_district)
        .unwrap()
        .profile();
    let second_district = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Second Command District".to_owned(),
            profile,
        },
    )
    .unwrap();
    let mut captains = Vec::new();
    for (index, district) in [first_district, second_district].into_iter().enumerate() {
        let captain = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: format!("District Captain {index}"),
                organization: Some(fixture.organization),
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
            &fixture.state,
            MandateDraft {
                organization: fixture.organization,
                manager: captain,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
        captains.push(MandateAuthority {
            mandate,
            manager: captain,
            scope,
        });
    }
    let prospective = [
        ProjectedCommandJob {
            organization: fixture.organization,
            captain: captains[0].manager,
            kind: EnterpriseKind::Protection,
            location: fixture.location,
            associates: EnterpriseKind::Protection.baseline_associates(),
            supporting_businesses: 0,
        },
        ProjectedCommandJob {
            organization: fixture.organization,
            captain: captains[1].manager,
            kind: EnterpriseKind::Protection,
            location: EnterpriseLocation::Neighborhood(second_district),
            associates: EnterpriseKind::Protection.baseline_associates(),
            supporting_businesses: 0,
        },
    ];
    // Each same-minute NPC expansion is individually feasible. A combined
    // batch must not book two direct command lines against one weak executive.
    assert!(projected_command_jobs_fit(
        &fixture.state,
        &prospective[..1]
    ));
    assert!(projected_command_jobs_fit(
        &fixture.state,
        &prospective[1..]
    ));
    assert!(!projected_command_jobs_fit(&fixture.state, &prospective));
    let first = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: captains[0],
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let second_draft = EnterpriseDraft {
        kind: EnterpriseKind::Protection,
        organization: fixture.organization,
        authority: captains[1],
        location: EnterpriseLocation::Neighborhood(second_district),
        supporting_businesses: BTreeSet::new(),
        cash_account: fixture.cash,
        settlement_account: second_settlement,
    };
    let boss_load = manager_workload(&fixture.state, executive).unwrap();
    assert_eq!(
        (
            boss_load.command_lines,
            boss_load.required,
            boss_load.available
        ),
        (1, 1, 1)
    );
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_establish_enterprise(&registry, &fixture.state, second_draft.clone()).err(),
        Some(EnterpriseError::ManagerWorkloadExceeded {
            manager: executive,
            required: 2,
            available: 1,
        })
    );
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    // The extra member starts idle. Only real reporting-line reassignment
    // lets one skilled underboss command both independent field captains.
    let underboss = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Field Underboss".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(90).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    assert_eq!(
        validate_establish_enterprise(&registry, &fixture.state, second_draft.clone()).err(),
        Some(EnterpriseError::ManagerWorkloadExceeded {
            manager: executive,
            required: 2,
            available: 1,
        })
    );
    for captain in &captains {
        crate::world::world_system::validate_set_character_supervisor(
            &fixture.state,
            captain.manager,
            Some(underboss),
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    }
    assert!(projected_command_jobs_fit(
        &fixture.state,
        &prospective[1..],
    ));
    let second = validate_establish_enterprise(&registry, &fixture.state, second_draft)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        manager_workload(&fixture.state, executive)
            .unwrap()
            .command_lines,
        1
    );
    let underboss_load = manager_workload(&fixture.state, underboss).unwrap();
    assert_eq!(
        (
            underboss_load.command_lines,
            underboss_load.required,
            underboss_load.available
        ),
        (2, 2, 30)
    );

    // A supervisor reshuffle can temporarily overload the boss between ticks.
    // The following day closes the smallest excess reporting branch through
    // the ordinary racket lifecycle, rather than retaining fictional control.
    crate::world::world_system::validate_set_character_supervisor(
        &fixture.state,
        captains[1].manager,
        Some(executive),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    assert_eq!(
        manager_workload(&fixture.state, executive)
            .unwrap()
            .required,
        2
    );
    // A weak intermediate supervisor cannot manage a tightly controlled branch.
    // Resolve that branch before the executive, whose overload disappears with it.
    let mut nested = fixture.state.clone();
    let weak = insert_character(
        &mut nested,
        CharacterDraft {
            name: "Inexperienced Supervisor".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    crate::world::world_system::validate_set_character_supervisor(
        &nested,
        captains[0].manager,
        Some(weak),
    )
    .unwrap()
    .commit(&mut nested)
    .unwrap();
    crate::world::world_system::validate_set_character_autonomy(
        &nested,
        fixture.organization,
        captains[0].manager,
        AutonomyLevel::Guided,
    )
    .unwrap()
    .commit(&mut nested)
    .unwrap();
    let load = manager_workload(&nested, weak).unwrap();
    assert_eq!((load.required, load.available), (2, 1));
    nested.advance_clock(SimDuration::from_minutes(1_440));
    let mut replay = restore_save(&registry, build_save(&registry, &nested).unwrap()).unwrap();
    for state in [&mut nested, &mut replay] {
        assert_eq!(
            apply_due_overstretched_manager_suspensions(state),
            vec![first],
            "a child overload must not sacrifice a healthy sibling branch"
        );
        assert_eq!(
            state.enterprises().get_enterprise(second).unwrap().status(),
            EnterpriseStatus::Active
        );
        let load = manager_workload(state, executive).unwrap();
        assert_eq!((load.required, load.available), (1, 1));
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&nested).unwrap(),
        bincode::serialize(&replay).unwrap()
    );

    assert!(apply_due_overstretched_manager_suspensions(&mut fixture.state).is_empty());
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(
        apply_due_overstretched_manager_suspensions(&mut fixture.state),
        vec![second]
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(first)
            .unwrap()
            .status(),
        EnterpriseStatus::Active
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(second)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(
        manager_workload(&restored, executive),
        manager_workload(&fixture.state, executive)
    );
    validate_state_against_registry(&registry, &restored).unwrap();

    // Both branches can individually reopen, but the shared executive can
    // supervise only one. A funded daily recovery cohort must not panic or
    // exceed the same command capacity that governs fresh establishments.
    validate_suspend_enterprise(&fixture.state, first)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    fund_enterprise_fixture_cash(&mut fixture, 200_000);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    for enterprise in [first, second] {
        assert!(validate_resume_enterprise(&registry, &fixture.state, enterprise).is_ok());
    }
    let outcome =
        crate::enterprises::autonomous_lifecycle::apply_due_autonomous_enterprise_lifecycle(
            &registry,
            &mut fixture.state,
        )
        .unwrap();
    assert_eq!(outcome.resumed, vec![first]);
    assert_eq!(
        outcome.resumed_mandates,
        BTreeSet::from([captains[0].mandate])
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(second)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );
    assert_eq!(
        manager_workload(&fixture.state, executive)
            .unwrap()
            .required,
        1
    );
    validate_invariants(&fixture.state);
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn arrested_captain_forces_real_racket_shutdown_without_a_magic_staffing_buffer() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let racket = establish_protection(&registry, &mut fixture);
    let captain = fixture.authority.manager;
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Command Disruption Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .unwrap();
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: police,
            title: "Captain custody case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(captain)]),
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let mut evidence = BTreeSet::new();
    for (kind, strength) in [
        (EvidenceKind::FinancialRecord, EvidenceStrength::Strong),
        (EvidenceKind::Document, EvidenceStrength::Corroborating),
    ] {
        evidence.insert(
            validate_add_evidence(
                &fixture.state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject: EntityRef::Character(captain),
                    origin: None,
                    kind,
                    strength,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: fixture.state.now(),
                },
            )
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap(),
        );
    }
    validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: captain,
            investigation,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let workload = manager_workload(&fixture.state, captain).unwrap();
    assert_eq!(workload.required, 0);
    assert_eq!(workload.available, 0);
    assert!(apply_due_overstretched_manager_suspensions(&mut fixture.state).is_empty());
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(apply_due_overstretched_manager_suspensions(&mut fixture.state).is_empty());
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(racket)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(racket)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );
    validate_state_against_registry(&registry, &restored).unwrap();
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
