use super::*;

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
            location,
            20,
            0,
            Some(enterprise)
        )
        .is_ok()
    );
    assert!(matches!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            location,
            25,
            0,
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
            location,
            14,
            0,
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
            location,
            13,
            0,
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
            location,
            14,
            0,
            None
        ),
        Err(EnterpriseError::ManagerWorkloadExceeded { .. })
    ));
    assert!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            additional_captain,
            location,
            14,
            0,
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
