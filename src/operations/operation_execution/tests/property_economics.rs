//! Operation property proceeds, disposition, depletion, and financial-reporting tests.

use super::*;

fn insert_property_disposition_fixture(
    registry: &Registry,
    state: &mut AppState,
    neighborhood: NeighborhoodId,
    organization: OrganizationId,
) -> (BusinessId, FinancialAccountId, FinancialAccountId) {
    let resale_venue = insert_business(
        registry,
        state,
        BusinessDraft {
            name: "Fixture Pawn Exchange".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::ResaleMarket,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("resale venue should validate");
    let cash_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("liquidation cash account should validate");
    let settlement_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("liquidation settlement account should validate");
    (resale_venue, cash_account, settlement_account)
}

#[test]
fn property_acquisition_persists_estimated_held_value_with_partial_recovery() {
    let (registry, mut achieved_state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let start = run_tick(&registry, &mut achieved_state);
    assert_eq!(start.started_operations, vec![operation]);
    achieved_state.advance_clock(SimDuration::from_minutes(45));
    let mut partial_state = achieved_state.clone();

    let achieved_plan = decide_operation_resolution(
        &registry,
        &achieved_state,
        operation,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("favorable property operation should resolve");
    assert_eq!(
        achieved_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    let achieved_proceeds = achieved_plan
        .outcome
        .property_proceeds_plan
        .proceeds
        .expect("achieved property acquisition should create held proceeds");
    assert_eq!(achieved_proceeds.estimated_value().cents(), 56_400);
    assert!(
        achieved_plan
            .narrative
            .summary
            .contains("estimated held value of $564.00")
    );
    assert!(
        achieved_plan
            .narrative
            .summary
            .contains("was held for later liquidation")
    );
    validate_operation_resolution_plan(&registry, &achieved_state, achieved_plan)
        .expect("achieved property proceeds should validate")
        .commit(&mut achieved_state)
        .expect("achieved property proceeds should commit");
    assert_eq!(
        achieved_state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.property_proceeds())
            .map(|proceeds| proceeds.estimated_value().cents()),
        Some(56_400)
    );
    let organization = achieved_state
        .operations()
        .get_operation(operation)
        .expect("completed property operation should persist")
        .responsible_organization();
    let financial_report = validate_organization_financial_report(
        &achieved_state,
        organization,
        SimTime::ZERO,
        achieved_state.now(),
    )
    .expect("held property should integrate into organization financial reporting")
    .commit(&mut achieved_state)
    .expect("held property financial report should commit");
    let report = achieved_state
        .reports()
        .get_report(financial_report)
        .expect("organization financial report should persist");
    assert!(report.entries()[0].summary.contains(
        "Held operation property at period end: 1 operation(s), estimated value $564.00"
    ));
    assert!(report.entries().iter().any(|entry| {
        entry.entities.contains(&EntityRef::Operation(operation))
            && entry.summary.contains("estimated held value of $564.00")
    }));

    let (resale_venue, cash_account, settlement_account) = insert_property_disposition_fixture(
        &registry,
        &mut achieved_state,
        neighborhood,
        organization,
    );
    let disposition = validate_dispose_property(
        &registry,
        &achieved_state,
        PropertyDispositionDraft {
            operation,
            venue: resale_venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("held burglary property should be disposable through a resale venue");
    assert_eq!(disposition.realized_value().cents(), 32_148);
    let disposition_outcome = disposition
        .commit(&mut achieved_state)
        .expect("property disposition should commit atomically");
    assert_eq!(disposition_outcome.realized_value.cents(), 32_148);
    assert_eq!(
        achieved_state
            .finance()
            .get_account(cash_account)
            .expect("cash account should persist")
            .balance()
            .cents(),
        32_148
    );
    assert_eq!(
        achieved_state
            .finance()
            .get_account(settlement_account)
            .expect("settlement account should persist")
            .balance()
            .cents(),
        -32_148
    );
    assert!(matches!(
      validate_dispose_property(
        &registry,
        &achieved_state,
        PropertyDispositionDraft {
          operation,
          venue: resale_venue,
          cash_account,
          settlement_account,
        },
      ),
      Err(PropertyDispositionError::AlreadyDisposed(found)) if found == operation
    ));
    let liquidated_report = validate_organization_financial_report(
        &achieved_state,
        organization,
        SimTime::ZERO,
        achieved_state.now(),
    )
    .expect("liquidated property should integrate into organization financial reporting")
    .commit(&mut achieved_state)
    .expect("liquidated property financial report should commit");
    let liquidated_report = achieved_state
        .reports()
        .get_report(liquidated_report)
        .expect("liquidation financial report should persist");
    assert!(
        liquidated_report.entries()[0].summary.contains(
            "Held operation property at period end: 0 operation(s), estimated value $0.00"
        )
    );
    assert!(liquidated_report.entries()[0].summary.contains(
        "Liquidated operation property during period: 1 disposition(s), realized cash $321.48"
    ));
    assert!(liquidated_report.entries().iter().any(|entry| {
        entry.entities.contains(&EntityRef::Operation(operation))
            && entry
                .summary
                .contains("liquidated through Fixture Pawn Exchange")
            && entry.summary.contains("$321.48")
    }));
    let manager = achieved_state
        .operations()
        .get_operation(operation)
        .expect("completed property operation should persist before enterprise establishment")
        .leader();
    let scope = crate::delegation::ResponsibilityScope::Neighborhood(neighborhood);
    let mandate = crate::delegation::delegation_system::validate_assign_mandate(
        &achieved_state,
        crate::delegation::MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("completed property crew leader should accept a district mandate")
    .commit(&mut achieved_state)
    .expect("property-disposition district mandate should commit");
    crate::enterprises::enterprise_execution::validate_establish_enterprise(
        &registry,
        &achieved_state,
        crate::enterprises::EnterpriseDraft {
            kind: crate::enterprises::EnterpriseKind::Protection,
            organization,
            authority: crate::delegation::MandateAuthority {
                mandate,
                manager,
                scope,
            },
            location: crate::enterprises::EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .expect("historical property liquidation must not permanently reserve its settlement account")
    .commit(&mut achieved_state)
    .expect("later enterprise account dedication should commit after property liquidation");

    let restored = restore_save(
        &registry,
        build_save(&registry, &achieved_state).expect("property disposition state should save"),
    )
    .expect("property disposition state should restore");
    let restored_disposition = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.property_disposition())
        .expect("property disposition should survive save restoration");
    assert_eq!(restored_disposition.realized_value().cents(), 32_148);
    assert_eq!(restored_disposition.venue(), resale_venue);
    validate_state_against_registry(&registry, &restored)
        .expect("restored property disposition should remain registry-valid");
    validate_invariants(&restored);

    let partial_plan = decide_operation_resolution(
        &registry,
        &partial_state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("neutral property operation should resolve");
    assert_eq!(
        partial_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Partial
    );
    assert_eq!(
        partial_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .expect("partial property acquisition should create reduced held proceeds")
            .estimated_value()
            .cents(),
        22_560
    );
    validate_operation_resolution_plan(&registry, &partial_state, partial_plan)
        .expect("partial property proceeds should validate")
        .commit(&mut partial_state)
        .expect("partial property proceeds should commit");
    validate_state_against_registry(&registry, &achieved_state)
        .expect("achieved property proceeds should remain registry-valid");
    validate_state_against_registry(&registry, &partial_state)
        .expect("partial property proceeds should remain registry-valid");
    validate_invariants(&achieved_state);
    validate_invariants(&partial_state);
}

#[test]
fn repeat_scores_on_one_target_deplete_and_recover_after_the_recency_window() {
    let (registry, mut state, _police, _neighborhood, first) =
        make_exposed_business_operation_fixture(false);
    let organization = state
        .operations()
        .get_operation(first)
        .expect("first operation should persist")
        .responsible_organization();
    let (business, leader, specialist) = {
        let record = state
            .operations()
            .get_operation(first)
            .expect("first operation should persist");
        let OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } = record.objective()
        else {
            panic!("fixture operation must target business property");
        };
        let specialist = *record
            .roles()
            .get(&RoleKind::EntrySpecialist)
            .expect("fixture entry specialist should persist");
        (*business, record.leader(), specialist)
    };

    let authorize_follow_up =
        |registry: &Registry, state: &mut AppState, title: &str| -> OperationId {
            validate_authorize_operation(
                registry,
                state,
                OperationDraft {
                    title: title.to_owned(),
                    kind: OperationKind::Burglary,
                    responsible_organization: organization,
                    leader,
                    objective: OperationObjective::AcquireProperty {
                        target: EntityRef::Business(business),
                    },
                    approach: OperationApproach::Covert,
                    roles: BTreeMap::from([
                        (RoleKind::Coordinator, leader),
                        (RoleKind::EntrySpecialist, specialist),
                    ]),
                    intelligence: BTreeSet::new(),
                    constraints: Vec::new(),
                    contingencies: Vec::new(),
                    scheduled_for: state.now() + SimDuration::ONE_MINUTE,
                },
            )
            .expect("follow-up burglary should validate")
            .commit(state)
            .expect("follow-up burglary should commit")
        };

    let resolve_achieved = |registry: &Registry,
                            state: &mut AppState,
                            operation: OperationId|
     -> OperationResolutionPlan {
        run_tick(registry, state);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            registry,
            state,
            operation,
            OperationResolutionRandomness::new(12, 0),
        )
        .expect("favorable property operation should resolve");
        assert_eq!(
            plan.outcome.objective_outcome,
            OperationObjectiveOutcome::Achieved
        );
        validate_operation_resolution_plan(registry, state, plan.clone())
            .expect("resolution should validate")
            .commit(state)
            .expect("resolution should commit");
        plan
    };

    // The first take yields full value with no depletion note.
    run_tick(&registry, &mut state);
    assert_eq!(
        state.operations().get_operation(first).map(|r| r.status()),
        Some(OperationStatus::InProgress)
    );
    state.advance_clock(SimDuration::from_minutes(45));
    let first_plan = decide_operation_resolution(
        &registry,
        &state,
        first,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("first take should resolve");
    assert_eq!(
        first_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    assert_eq!(
        first_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .as_ref()
            .expect("first take should create proceeds")
            .estimated_value()
            .cents(),
        56_400
    );
    assert!(
        !first_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );
    assert!(!first_plan.narrative.summary.contains("lighter than usual"));
    validate_operation_resolution_plan(&registry, &state, first_plan)
        .expect("first take should validate")
        .commit(&mut state)
        .expect("first take should commit");

    // An immediate second score on the same target finds partially replaced stock.
    let second = authorize_follow_up(&registry, &mut state, "Repeat burglary");
    let second_plan = resolve_achieved(&registry, &mut state, second);
    let second_value = second_plan
        .outcome
        .property_proceeds_plan
        .proceeds
        .as_ref()
        .expect("second take should create reduced proceeds")
        .estimated_value()
        .cents();
    assert!(
        (28_200..56_400).contains(&second_value),
        "the target should replenish slightly during the follow-up operation, but remain depleted"
    );
    assert!(
        second_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );
    assert!(second_plan.narrative.summary.contains("lighter than usual"));

    let recovery_window = registry
        .get_operation(OperationKind::Burglary)
        .execution()
        .property_proceeds()
        .expect("burglary must define property proceeds")
        .recent_take_recovery_window();
    let mut half_recovered = state.clone();
    let mut fully_recovered = state;

    // Halfway through replenishment, both prior scores still reduce the target, but elapsed time
    // must restore materially more value than the immediate follow-up received.
    half_recovered.advance_clock(SimDuration::from_minutes(recovery_window.as_minutes() / 2));
    let third = authorize_follow_up(&registry, &mut half_recovered, "Recovering burglary");
    let third_plan = resolve_achieved(&registry, &mut half_recovered, third);
    let third_value = third_plan
        .outcome
        .property_proceeds_plan
        .proceeds
        .as_ref()
        .expect("partially recovered take should create proceeds")
        .estimated_value()
        .cents();
    assert!(
        third_value > second_value && third_value < 56_400,
        "recovery must be gradual and monotonic before the authored window ends"
    );
    assert!(
        third_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );

    // Once the authored recovery window has elapsed, the old scores no longer suppress value.
    fully_recovered.advance_clock(recovery_window);
    let fourth = authorize_follow_up(&registry, &mut fully_recovered, "Recovered burglary");
    let fourth_plan = resolve_achieved(&registry, &mut fully_recovered, fourth);
    assert_eq!(
        fourth_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .as_ref()
            .expect("recovered take should create full proceeds")
            .estimated_value()
            .cents(),
        56_400
    );
    assert!(
        !fourth_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );

    validate_state_against_registry(&registry, &half_recovered)
        .expect("partially recovered take history should remain registry-valid");
    validate_state_against_registry(&registry, &fully_recovered)
        .expect("fully recovered take history should remain registry-valid");
    validate_invariants(&half_recovered);
    validate_invariants(&fully_recovered);
}

#[test]
fn zero_value_repeat_score_does_not_create_phantom_depletion() {
    let (registry, mut state, _police, _neighborhood, first) =
        make_exposed_business_operation_fixture(false);
    let organization = state
        .operations()
        .get_operation(first)
        .expect("first operation should persist")
        .responsible_organization();
    let (business, leader, specialist) = {
        let record = state
            .operations()
            .get_operation(first)
            .expect("first operation should persist");
        let OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } = record.objective()
        else {
            panic!("fixture operation must target business property");
        };
        (
            *business,
            record.leader(),
            *record
                .roles()
                .get(&RoleKind::EntrySpecialist)
                .expect("fixture entry specialist should persist"),
        )
    };
    let recovery_window = registry
        .get_operation(OperationKind::Burglary)
        .execution()
        .property_proceeds()
        .expect("burglary must define property proceeds")
        .recent_take_recovery_window();

    let authorize = |state: &mut AppState, title: String| -> OperationId {
        validate_authorize_operation(
            &registry,
            state,
            OperationDraft {
                title,
                kind: OperationKind::Burglary,
                responsible_organization: organization,
                leader,
                objective: OperationObjective::AcquireProperty {
                    target: EntityRef::Business(business),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([
                    (RoleKind::Coordinator, leader),
                    (RoleKind::EntrySpecialist, specialist),
                ]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("repeat burglary should validate")
        .commit(state)
        .expect("repeat burglary should commit")
    };

    let resolve = |state: &mut AppState, operation: OperationId| -> bool {
        run_tick(&registry, state);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            &registry,
            state,
            operation,
            OperationResolutionRandomness::new(12, 0),
        )
        .expect("repeat burglary should resolve");
        let has_proceeds = plan.outcome.property_proceeds_plan.proceeds.is_some();
        validate_operation_resolution_plan(&registry, state, plan)
            .expect("repeat burglary resolution should validate")
            .commit(state)
            .expect("repeat burglary resolution should commit");
        has_proceeds
    };

    run_tick(&registry, &mut state);
    state.advance_clock(SimDuration::from_minutes(45));
    let first_plan = decide_operation_resolution(
        &registry,
        &state,
        first,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("first burglary should resolve");
    validate_operation_resolution_plan(&registry, &state, first_plan)
        .expect("first burglary should validate")
        .commit(&mut state)
        .expect("first burglary should commit");

    let mut zero_value_operation = None;
    for sequence in 0..32 {
        let operation = authorize(&mut state, format!("Depletion probe {sequence}"));
        if !resolve(&mut state, operation) {
            zero_value_operation = Some(operation);
            break;
        }
    }
    let zero_value_operation =
        zero_value_operation.expect("repeated successful scores must eventually exhaust cents");
    assert!(
        state
            .operations()
            .get_operation(zero_value_operation)
            .and_then(|record| record.resolution())
            .is_some_and(|resolution| resolution.property_proceeds().is_none()),
        "the exhaustion probe must be a completed tactical success with no property removed"
    );

    let next = authorize(&mut state, "Post-empty index probe".to_owned());
    let indexed = state.operations.recent_successful_take_times(
        business,
        OperationKind::Burglary,
        state.now(),
        recovery_window,
        next,
    );
    let positive_recent = state
        .operations()
        .operations_for_organization(organization)
        .filter_map(|record| record.resolution())
        .filter(|resolution| {
            resolution.property_proceeds().is_some()
                && state
                    .now()
                    .as_minutes()
                    .saturating_sub(resolution.resolved_at().as_minutes())
                    < u64::from(recovery_window.as_minutes())
        })
        .count();
    assert_eq!(
        indexed.len(),
        positive_recent,
        "zero-value tactical successes must not become phantom depletion events"
    );
    validate_state_against_registry(&registry, &state)
        .expect("zero-value take history should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn property_disposition_reporting_respects_executive_brief_window() {
    let (registry, mut state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("authorized operation should persist")
        .responsible_organization();
    designate_player_organization(&mut state, organization)
        .expect("test organization should be designatable as player");
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("favorable property operation should resolve");
    assert_eq!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("property acquisition should validate")
        .commit(&mut state)
        .expect("property acquisition should commit");

    let (venue, cash_account, settlement_account) =
        insert_property_disposition_fixture(&registry, &mut state, neighborhood, organization);
    let mut same_window = state.clone();
    let mut later_window = state;

    validate_dispose_property(
        &registry,
        &same_window,
        PropertyDispositionDraft {
            operation,
            venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("same-window property disposition should validate")
    .commit(&mut same_window)
    .expect("same-window property disposition should commit");
    let delta = 1_439_u64
        .checked_sub(same_window.now().as_minutes())
        .expect("fixture should resolve before first daily brief");
    same_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("first brief delta should fit SimDuration"),
    ));
    let same_window_tick = run_tick(&registry, &mut same_window);
    let same_window_brief = same_window_tick
        .executive_brief
        .expect("first daily brief should be generated");
    let same_window_report = same_window
        .reports()
        .get_report(same_window_brief)
        .expect("same-window executive brief should persist");
    let operation_entries = same_window_report
        .entries()
        .iter()
        .filter(|entry| entry.entities.contains(&EntityRef::Operation(operation)))
        .collect::<Vec<_>>();
    assert_eq!(operation_entries.len(), 2);
    assert!(
        operation_entries
            .iter()
            .any(|entry| entry.summary.contains("was held for later liquidation"))
    );
    assert!(operation_entries.iter().any(|entry| {
        entry.summary.starts_with("Property from ")
            && entry
                .summary
                .contains("liquidated through Fixture Pawn Exchange for $321.48")
    }));

    let delta = 1_439_u64
        .checked_sub(later_window.now().as_minutes())
        .expect("fixture should resolve before first daily brief");
    later_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("first brief delta should fit SimDuration"),
    ));
    let first_tick = run_tick(&registry, &mut later_window);
    let first_brief = first_tick
        .executive_brief
        .expect("first daily brief should be generated");
    let first_report = later_window
        .reports()
        .get_report(first_brief)
        .expect("first executive brief should persist");
    assert!(
        first_report
            .entries()
            .iter()
            .any(|entry| entry.summary.contains("was held for later liquidation"))
    );

    validate_dispose_property(
        &registry,
        &later_window,
        PropertyDispositionDraft {
            operation,
            venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("later-window property disposition should validate")
    .commit(&mut later_window)
    .expect("later-window property disposition should commit");
    let delta = 2_879_u64
        .checked_sub(later_window.now().as_minutes())
        .expect("disposition should precede the second daily brief");
    later_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("second brief delta should fit SimDuration"),
    ));
    let second_tick = run_tick(&registry, &mut later_window);
    let second_brief = second_tick
        .executive_brief
        .expect("second daily brief should be generated");
    let second_report = later_window
        .reports()
        .get_report(second_brief)
        .expect("second executive brief should persist");
    assert!(second_report.entries().iter().any(|entry| {
        entry.summary.starts_with("Property from ")
            && entry
                .summary
                .contains("liquidated through Fixture Pawn Exchange for $321.48")
    }));
    assert!(
        !second_report
            .entries()
            .iter()
            .any(|entry| entry.summary.contains("was held for later liquidation"))
    );

    validate_state_against_registry(&registry, &same_window)
        .expect("same-window brief state should remain registry-valid");
    validate_state_against_registry(&registry, &later_window)
        .expect("later-window brief state should remain registry-valid");
    validate_invariants(&same_window);
    validate_invariants(&later_window);
}

#[test]
fn same_minute_post_disposition_venue_transfer_preserves_save_restore() {
    let (registry, mut state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("favorable property operation should resolve");
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("property operation should validate")
        .commit(&mut state)
        .expect("property operation should commit");
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("completed property operation should persist")
        .responsible_organization();
    let (venue, cash_account, settlement_account) =
        insert_property_disposition_fixture(&registry, &mut state, neighborhood, organization);
    validate_dispose_property(
        &registry,
        &state,
        PropertyDispositionDraft {
            operation,
            venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("held property should be disposable through the owned resale venue")
    .commit(&mut state)
    .expect("property disposition should commit");
    let disposition_version = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.property_disposition())
        .expect("property disposition should persist")
        .venue_version();

    // World ownership may change later in this same minute. The disposition already froze the
    // exact venue version it used, while cross-domain sub-minute ordering is intentionally not
    // persisted. Restore must therefore not reinterpret the final owner at this timestamp as if
    // it had necessarily preceded the liquidation.
    validate_transfer_business_ownership(&state, venue, BusinessOwner::Independent)
        .expect("resale venue should be transferable after disposition")
        .commit(&mut state)
        .expect("same-minute post-disposition transfer should commit");
    assert!(
        state
            .world()
            .get_business(venue)
            .expect("transferred venue should persist")
            .version()
            > disposition_version
    );
    validate_state(&state).expect("same-minute disposition transfer should remain valid state");
    validate_state_against_registry(&registry, &state)
        .expect("pinned disposition ownership version should remain registry-valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("same-minute disposition transfer should save"),
    )
    .expect("same-minute disposition transfer should restore");
    let restored_disposition = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.property_disposition())
        .expect("restored property disposition should persist");
    assert_eq!(restored_disposition.venue(), venue);
    assert_eq!(restored_disposition.venue_version(), disposition_version);
    validate_invariants(&restored);
}
