//! Police-response, patrol, jurisdiction, and operation-resolution integration tests.

use super::*;

#[derive(Clone, Serialize)]
struct PoliceResponseRoutingWire {
    authority: OrganizationId,
    neighborhood: NeighborhoodId,
    source_operation: OperationId,
}

#[derive(Clone, Serialize)]
struct PoliceResponseTimingWire {
    dispatched_at: SimTime,
    arrival_due_at: SimTime,
    arrived_at: Option<SimTime>,
}

#[derive(Clone, Serialize)]
struct PoliceResponseStateWire {
    alert_score: i16,
    response_presence: crate::world::Rating,
    jurisdiction_version: u32,
    patrol: Option<PoliceResponsePatrolSnapshot>,
    status: PoliceResponseStatus,
    version: u32,
}

#[derive(Clone, Serialize)]
struct PoliceResponseRecordWire {
    id: PoliceResponseId,
    routing: PoliceResponseRoutingWire,
    timing: PoliceResponseTimingWire,
    state: PoliceResponseStateWire,
}

fn police_response_wire(record: &PoliceResponseRecord) -> PoliceResponseRecordWire {
    PoliceResponseRecordWire {
        id: record.id(),
        routing: PoliceResponseRoutingWire {
            authority: record.authority(),
            neighborhood: record.neighborhood(),
            source_operation: record.source_operation(),
        },
        timing: PoliceResponseTimingWire {
            dispatched_at: record.dispatched_at(),
            arrival_due_at: record.arrival_due_at(),
            arrived_at: record.arrived_at(),
        },
        state: PoliceResponseStateWire {
            alert_score: record.alert_score(),
            response_presence: record.response_presence(),
            jurisdiction_version: record.jurisdiction_version(),
            patrol: record.patrol(),
            status: record.status(),
            version: record.version(),
        },
    }
}

fn replace_serialized_police_response(
    envelope: SaveEnvelope,
    original: &PoliceResponseRecord,
    replacement: &PoliceResponseRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("police response should serialize");
    let mirror = police_response_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("police response mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement police response should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized police response must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout police response corruption must remain decodable")
}

#[test]
fn neighborhood_exposure_opens_jurisdiction_case_and_survives_save_round_trip() {
    let (registry, mut original, police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    for _ in 0..45 {
        let tick = run_tick(&registry, &mut original);
        assert!(tick.resolved_operations.is_empty());
    }
    assert_eq!(original.now(), SimTime::from_minutes(45));
    let envelope = build_save(&registry, &original)
        .expect("pre-exposure-resolution operation state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&registry, decoded)
        .expect("pre-exposure-resolution operation save should restore");

    let original_tick = run_tick(&registry, &mut original);
    let restored_tick = run_tick(&registry, &mut restored);
    assert_eq!(original_tick, restored_tick);
    assert_eq!(original_tick.resolved_operations, vec![operation]);
    for state in [&original, &restored] {
        let resolution = state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("exposed operation should resolve");
        assert!(matches!(
            resolution.exposure().level(),
            OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
        ));
        let investigation_id = resolution
            .exposure()
            .investigation()
            .expect("jurisdictional exposure should open an investigation");
        let investigation = state
            .legal()
            .get_investigation(investigation_id)
            .expect("operation investigation should persist");
        assert_eq!(investigation.owner(), police);
        assert_eq!(resolution.exposure().evidence().len(), 1);
        let legal_activity_information = resolution
            .legal_activity_information()
            .expect("jurisdictional exposure should create player legal-activity knowledge");
        let legal_activity = state
            .intelligence()
            .get_information(legal_activity_information)
            .expect("player legal-activity information should persist");
        assert_eq!(legal_activity.topic(), InformationTopic::LegalActivity);
        assert_eq!(legal_activity.subject(), EntityRef::Operation(operation));
        assert!(
            legal_activity
                .summary()
                .contains("produced a police investigation")
        );
        let evidence_id = *resolution
            .exposure()
            .evidence()
            .iter()
            .next()
            .expect("operation exposure should persist one evidence record");
        let evidence = state
            .legal()
            .get_evidence(evidence_id)
            .expect("operation evidence should persist");
        assert_eq!(evidence.origin(), Some(EntityRef::Operation(operation)));
        assert_eq!(
            state
                .legal()
                .all_evidence()
                .filter(|record| record.origin() == Some(EntityRef::Operation(operation)))
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![evidence_id]
        );
        validate_state(state).expect("exposure-linked legal state should validate");
        validate_invariants(state);
    }
    let original_exposure = original
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("original exposure should resolve")
        .exposure();
    let restored_exposure = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("restored exposure should resolve")
        .exposure();
    assert_eq!(original_exposure.level(), restored_exposure.level());
    assert_eq!(original_exposure.score(), restored_exposure.score());
    assert_eq!(original_exposure.factors(), restored_exposure.factors());
    assert_eq!(
        original_exposure.investigation(),
        restored_exposure.investigation()
    );
    assert_eq!(original_exposure.evidence(), restored_exposure.evidence());
    assert_eq!(
        original
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information()),
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information())
    );
}

#[test]
fn exposed_operation_without_jurisdiction_creates_no_implicit_case() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    for _ in 0..46 {
        run_tick(&registry, &mut state);
    }
    let exposure = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("exposed operation should resolve")
        .exposure();
    assert!(matches!(
        exposure.level(),
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ));
    assert_eq!(exposure.investigation(), None);
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information()),
        None
    );
    assert!(exposure.evidence().is_empty());
    assert_eq!(
        state
            .legal()
            .all_evidence()
            .filter(|record| record.origin() == Some(EntityRef::Operation(operation)))
            .count(),
        0
    );
    validate_state(&state).expect("unrouted exposure should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn patrol_presence_controls_persisted_police_response_delay() {
    let (low_registry, mut low_state, low_police, low_neighborhood, low_operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &low_state,
        PatrolDeploymentDraft {
            organization: low_police,
            neighborhood: low_neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("zero-presence patrol should validate")
    .commit(&mut low_state)
    .expect("zero-presence patrol should commit");
    let low_start = run_tick(&low_registry, &mut low_state);
    assert_eq!(low_start.started_operations, vec![low_operation]);
    let low_response_id = low_state
        .operations()
        .get_operation(low_operation)
        .and_then(|record| record.police_response())
        .expect("observable burglary should dispatch a response");
    let low_response = low_state
        .legal()
        .get_police_response(low_response_id)
        .expect("low-presence response should persist");
    assert_eq!(low_response.response_presence().value(), 0);
    assert_eq!(
        low_response.arrival_due_at().as_minutes() - low_response.dispatched_at().as_minutes(),
        12
    );

    let (high_registry, mut high_state, high_police, high_neighborhood, high_operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &high_state,
        PatrolDeploymentDraft {
            organization: high_police,
            neighborhood: high_neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(100).expect("full patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("full-presence patrol should validate")
    .commit(&mut high_state)
    .expect("full-presence patrol should commit");
    let high_start = run_tick(&high_registry, &mut high_state);
    assert_eq!(high_start.started_operations, vec![high_operation]);
    let high_response_id = high_state
        .operations()
        .get_operation(high_operation)
        .and_then(|record| record.police_response())
        .expect("observable burglary should dispatch a response");
    let high_response = high_state
        .legal()
        .get_police_response(high_response_id)
        .expect("high-presence response should persist");
    assert_eq!(high_response.response_presence().value(), 100);
    assert_eq!(
        high_response.arrival_due_at().as_minutes() - high_response.dispatched_at().as_minutes(),
        3
    );

    validate_state_against_registry(&low_registry, &low_state)
        .expect("low-presence response state should match authored content");
    validate_state_against_registry(&high_registry, &high_state)
        .expect("high-presence response state should match authored content");
    validate_invariants(&low_state);
    validate_invariants(&high_state);
}

#[test]
fn police_arrival_before_entry_executes_standing_abort_contingency() {
    let (registry, mut state, police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
        );
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let response_id = operation_record
        .police_response()
        .expect("high-observation burglary should dispatch police response");
    let entry_at = operation_record
        .entry_at()
        .expect("burglary should have an authored entry milestone");

    let mut arrival_tick = None;
    while state.now() < entry_at {
        let outcome = run_tick(&registry, &mut state);
        if outcome.arrived_police_responses.contains(&response_id) {
            arrival_tick = Some(outcome.now);
            break;
        }
    }
    let arrived_at = arrival_tick.expect("police response should arrive before burglary entry");
    assert!(arrived_at < entry_at);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should persist");
    assert_eq!(operation_record.status(), OperationStatus::Aborted);
    let abort = operation_record
        .abort_record()
        .expect("standing police contingency should create abort history");
    assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
    assert_eq!(
        abort.cause(),
        OperationAbortCause::PoliceArrival(response_id)
    );
    assert!(operation_record.resolution().is_none());
    assert_eq!(
        state
            .legal()
            .get_police_response(response_id)
            .and_then(|response| response.arrived_at()),
        Some(arrived_at)
    );
    let mut participants = operation_record
        .roles()
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    participants.insert(operation_record.leader());
    for participant in participants {
        let pressure: Vec<_> = state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Character(participant),
                InformationTopic::PoliceActivity,
            )
            .collect();
        assert_eq!(pressure.len(), 1);
        assert_eq!(
            pressure[0].source_kind(),
            InformationSourceKind::DirectObservation
        );
        assert_eq!(
            pressure[0].source_entity(),
            Some(EntityRef::Organization(police))
        );
        assert_eq!(pressure[0].subject(), EntityRef::Character(participant));
        assert_eq!(pressure[0].observed_at(), arrived_at);
        assert_eq!(pressure[0].reliability(), Reliability::DirectAccess);
        assert_eq!(pressure[0].specificity(), Specificity::Precise);
    }
    validate_state(&state).expect("police-contingency abort state should remain valid");
    validate_state_against_registry(&registry, &state)
        .expect("police-contingency abort should match authored content");
    validate_invariants(&state);
}

#[test]
fn post_entry_police_arrival_raises_provenance_backed_decision() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("zero-presence patrol should validate")
    .commit(&mut state)
    .expect("zero-presence patrol should commit");

    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let response_id = operation_record
        .police_response()
        .expect("observable burglary should dispatch police response");
    let entry_at = operation_record
        .entry_at()
        .expect("burglary should have an authored entry milestone");
    let response_due = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    assert!(response_due > entry_at);

    let arrival_outcome = loop {
        let outcome = run_tick(&registry, &mut state);
        if outcome.arrived_police_responses.contains(&response_id) {
            break outcome;
        }
    };
    assert_eq!(arrival_outcome.now, response_due);
    assert_eq!(arrival_outcome.arrived_police_responses, vec![response_id]);
    assert_eq!(arrival_outcome.decision_requests.len(), 1);
    assert!(arrival_outcome.resolved_operations.is_empty());

    let decision_id = arrival_outcome.decision_requests[0].decision;
    let decision = state
        .decisions()
        .get_decision(decision_id)
        .expect("response decision should persist");
    assert_eq!(decision.requested_at(), response_due);
    assert!(matches!(
      decision.context(),
      DecisionContext::OperationPoliceArrival {
        operation: decision_operation,
        response: decision_response,
      } if decision_operation == operation && decision_response == response_id
    ));
    assert!(decision.summary().contains("response reached"));
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("decision-blocked operation should persist");
    assert_eq!(operation_record.status(), OperationStatus::AwaitingDecision);
    assert_eq!(
        operation_record.awaiting_decision_since(),
        Some(response_due)
    );

    let organization = operation_record.responsible_organization();
    let envelope = build_save(&registry, &state)
        .expect("pending police-arrival decision should survive save validation");
    let bytes =
        bincode::serialize(&envelope).expect("police-arrival decision save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("police-arrival decision save should deserialize");
    state = restore_save(&registry, decoded)
        .expect("pending police-arrival decision should restore with provenance indexes");
    assert_eq!(
        state
            .decisions()
            .decisions_for_operation(operation)
            .filter(|candidate| candidate.id() == decision_id)
            .count(),
        1
    );
    validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Continue,
    )
    .expect("post-entry police response should allow leadership to continue")
    .commit(&mut state)
    .expect("post-entry continue should resume operation");
    let resumed = state
        .operations()
        .get_operation(operation)
        .expect("resumed operation should persist");
    assert_eq!(resumed.status(), OperationStatus::InProgress);
    assert_eq!(resumed.awaiting_decision_since(), None);
    assert_eq!(
        state
            .legal()
            .get_police_response(response_id)
            .and_then(|response| response.arrived_at()),
        Some(response_due)
    );
    // One police response must never produce two leadership decisions: once its arrival
    // decision exists the operation is decision-blocked, and after resolution the response
    // is no longer a freshly-due dispatch.
    let duplicate = validate_request_police_arrival_decision_on_arrival(&state, response_id)
        .expect_err("one police response must not create duplicate leadership decisions");
    assert_eq!(
        duplicate,
        DecisionError::InvalidPoliceResponseDecision {
            operation,
            response: response_id,
        }
    );
    validate_state(&state).expect("post-entry response decision state should validate");
    validate_state_against_registry(&registry, &state)
        .expect("post-entry response decision should match authored content");
    validate_invariants(&state);
}

#[test]
fn police_arrival_decision_id_exhaustion_leaves_response_dispatched_and_operation_running() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("zero-presence patrol should validate")
    .commit(&mut state)
    .expect("zero-presence patrol should commit");
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("observable operation should dispatch a response");
    let response_due = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    let minutes_until_due = u32::try_from(response_due.as_minutes() - state.now().as_minutes())
        .expect("fixture response delay must fit SimDuration");
    state.advance_clock(SimDuration::from_minutes(minutes_until_due));
    let response_version = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .version();
    let operation_version = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist")
        .version();
    let information_next = state.ids.next_raw(IdKind::Information);
    state
        .ids
        .set_next_raw_for_test(IdKind::DecisionRequest, u32::MAX);

    let error = crate::operations::police_response_integration::apply_due_police_response_arrivals(
        &mut state,
    )
    .expect_err("decision allocator exhaustion must reject the whole response arrival");
    assert!(matches!(
        error,
        crate::operations::police_response_integration::PoliceResponseIntegrationError::IdExhaustion(
            IdExhaustionError::Exhausted {
                kind: "decision request",
                ..
            }
        )
    ));
    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("rejected response should persist as dispatched");
    assert_eq!(
        response.status(),
        crate::legal::PoliceResponseStatus::Dispatched
    );
    assert_eq!(response.arrived_at(), None);
    assert_eq!(response.version(), response_version);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("rejected response must retain operation");
    assert_eq!(operation_record.status(), OperationStatus::InProgress);
    assert_eq!(operation_record.awaiting_decision_since(), None);
    assert_eq!(operation_record.version(), operation_version);
    assert_eq!(state.ids.next_raw(IdKind::Information), information_next);
    assert!(
        state
            .decisions()
            .decisions_for_operation(operation)
            .next()
            .is_none()
    );
}

#[test]
fn police_arrival_abort_artifact_exhaustion_leaves_response_dispatched_and_operation_running() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
        );
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("observable operation should dispatch a response");
    let response_due = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    let entry_at = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.entry_at())
        .expect("burglary fixture should have an entry milestone");
    assert!(response_due < entry_at);
    let minutes_until_due = u32::try_from(response_due.as_minutes() - state.now().as_minutes())
        .expect("fixture response delay must fit SimDuration");
    state.advance_clock(SimDuration::from_minutes(minutes_until_due));
    let response_version = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .version();
    let operation_version = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist")
        .version();
    let information_next = state.ids.next_raw(IdKind::Information);
    let history_next = state.ids.next_raw(IdKind::HistoryEvent);
    state.ids.set_next_raw_for_test(IdKind::Report, u32::MAX);

    let error = crate::operations::police_response_integration::apply_due_police_response_arrivals(
        &mut state,
    )
    .expect_err("abort report exhaustion must reject the whole response arrival");
    assert!(matches!(
        error,
        crate::operations::police_response_integration::PoliceResponseIntegrationError::IdExhaustion(
            IdExhaustionError::Exhausted { kind: "report", .. }
        )
    ));
    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("rejected response should persist as dispatched");
    assert_eq!(
        response.status(),
        crate::legal::PoliceResponseStatus::Dispatched
    );
    assert_eq!(response.arrived_at(), None);
    assert_eq!(response.version(), response_version);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("rejected response must retain operation");
    assert_eq!(operation_record.status(), OperationStatus::InProgress);
    assert!(operation_record.abort_record().is_none());
    assert_eq!(operation_record.version(), operation_version);
    assert_eq!(state.ids.next_raw(IdKind::Information), information_next);
    assert_eq!(state.ids.next_raw(IdKind::HistoryEvent), history_next);
}

#[test]
fn arrived_response_penalizes_continuing_operation_and_stales_prearrival_plan() {
    let (registry, mut response_state, _police, _neighborhood, response_operation) =
        make_exposed_business_operation_fixture(true);
    let (_, mut control_state, _control_police, _control_neighborhood, control_operation) =
        make_exposed_business_operation_fixture(false);
    run_tick(&registry, &mut response_state);
    run_tick(&registry, &mut control_state);

    let response_id = response_state
        .operations()
        .get_operation(response_operation)
        .and_then(|record| record.police_response())
        .expect("jurisdictional burglary should dispatch response");
    response_state.advance_clock(SimDuration::from_minutes(45));
    control_state.advance_clock(SimDuration::from_minutes(45));
    let stale_plan = decide_operation_resolution(
        &registry,
        &response_state,
        response_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should be plannable before response processing");
    assert!(!stale_plan.outcome.factors.police_response_arrived());
    let response_outcome =
        crate::operations::police_response_integration::apply_due_police_response_arrivals(
            &mut response_state,
        )
        .expect("due response should process");
    assert_eq!(response_outcome.arrived, vec![response_id]);
    assert!(response_outcome.decisions.is_empty());
    let stale_error =
        match validate_operation_resolution_plan(&registry, &response_state, stale_plan) {
            Ok(_) => panic!("response arrival must invalidate a pre-arrival resolution plan"),
            Err(error) => error,
        };
    assert_eq!(
        stale_error,
        OperationResolutionError::StalePoliceResponseContext {
            operation: response_operation,
        }
    );

    let response_plan = decide_operation_resolution(
        &registry,
        &response_state,
        response_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("arrived-response operation should re-plan");
    let control_plan = decide_operation_resolution(
        &registry,
        &control_state,
        control_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("unrouted control operation should plan");
    assert!(response_plan.outcome.factors.police_response_arrived());
    assert!(!control_plan.outcome.factors.police_response_arrived());
    let execution = registry.get_operation(OperationKind::Burglary).execution();
    assert_eq!(
        control_plan.outcome.execution_margin - response_plan.outcome.execution_margin,
        i16::from(execution.police_arrival_difficulty_penalty())
    );
    assert_eq!(
        response_plan.outcome.exposure.score - control_plan.outcome.exposure.score,
        i16::from(execution.police_arrival_exposure_penalty())
    );
    validate_operation_resolution_plan(&registry, &response_state, response_plan)
        .expect("response-aware resolution should validate")
        .commit(&mut response_state)
        .expect("response-aware resolution should commit");
    validate_state_against_registry(&registry, &response_state)
        .expect("response-aware completion should validate against registry");
    validate_invariants(&response_state);
}

#[test]
fn police_response_arrival_is_deterministic_across_save_round_trip() {
    let (registry, mut original, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    run_tick(&registry, &mut original);
    let response_id = original
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("jurisdictional burglary should dispatch response");
    let due_at = original
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    while original.now() + SimDuration::ONE_MINUTE < due_at {
        let outcome = run_tick(&registry, &mut original);
        assert!(outcome.arrived_police_responses.is_empty());
    }
    let envelope =
        build_save(&registry, &original).expect("pre-arrival police response state should save");
    let bytes = bincode::serialize(&envelope).expect("response save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("response save should deserialize");
    let mut restored = restore_save(&registry, decoded).expect("response save should restore");

    let original_tick = run_tick(&registry, &mut original);
    let restored_tick = run_tick(&registry, &mut restored);
    assert_eq!(original_tick.arrived_police_responses, vec![response_id]);
    assert_eq!(restored_tick.arrived_police_responses, vec![response_id]);
    assert_eq!(
        original
            .legal()
            .get_police_response(response_id)
            .and_then(|record| record.arrived_at()),
        restored
            .legal()
            .get_police_response(response_id)
            .and_then(|record| record.arrived_at())
    );
    validate_state(&restored).expect("restored police-response state should validate");
    validate_invariants(&restored);
}

#[test]
fn same_minute_patrol_revision_does_not_invalidate_dispatch_snapshot() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(20).expect("fixture patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("initial patrol should validate")
    .commit(&mut state)
    .expect("initial patrol should commit");

    let started = run_tick(&registry, &mut state);
    assert_eq!(started.now, SimTime::from_minutes(1));
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("observable operation should dispatch a response");
    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("dispatch snapshot should persist");
    assert_eq!(
        response.patrol(),
        Some(PoliceResponsePatrolSnapshot::new(deployment, 1))
    );
    assert_eq!(response.response_presence().value(), 20);

    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![
            PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(80).expect("revised patrol presence should validate"),
            )
            .expect("revised patrol window should validate"),
        ],
    )
    .expect("same-minute patrol revision should validate")
    .commit(&mut state)
    .expect("same-minute patrol revision should commit after dispatch");
    assert_eq!(
        state
            .legal()
            .get_patrol_deployment(deployment)
            .expect("revised deployment should persist")
            .version(),
        2
    );

    validate_state(&state).expect("same-minute dispatch/revision ordering should remain valid");
    validate_state_against_registry(&registry, &state)
        .expect("later work in the dispatch minute must not rewrite its frozen patrol snapshot");
    let restored = restore_save(
        &registry,
        build_save(&registry, &state)
            .expect("same-minute historical patrol snapshot should remain saveable"),
    )
    .expect("same-minute historical patrol snapshot should restore");
    let restored_response = restored
        .legal()
        .get_police_response(response_id)
        .expect("restored response should persist");
    assert_eq!(
        restored_response.patrol(),
        Some(PoliceResponsePatrolSnapshot::new(deployment, 1))
    );
    assert_eq!(restored_response.response_presence().value(), 20);
}

#[test]
fn same_minute_jurisdiction_revision_does_not_invalidate_dispatch_snapshot() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.now, SimTime::from_minutes(1));
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("observable operation should dispatch a response");
    assert_eq!(
        state
            .legal()
            .get_police_response(response_id)
            .expect("dispatch snapshot should persist")
            .jurisdiction_version(),
        1
    );

    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(81)
                .expect("revised jurisdiction priority should validate"),
        },
    )
    .expect("same-minute jurisdiction revision should validate")
    .commit(&mut state)
    .expect("same-minute jurisdiction revision should commit after dispatch");
    assert_eq!(
        state
            .legal()
            .get_jurisdiction(police)
            .expect("revised jurisdiction should persist")
            .version(),
        2
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &state)
            .expect("same-minute historical jurisdiction snapshot should remain saveable"),
    )
    .expect("same-minute historical jurisdiction snapshot should restore");
    assert_eq!(
        restored
            .legal()
            .get_police_response(response_id)
            .expect("restored response should persist")
            .jurisdiction_version(),
        1
    );
}

#[test]
fn restore_rejects_police_response_jurisdiction_revision_created_after_dispatch() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("operation should dispatch a response");

    state.advance_clock(SimDuration::ONE_MINUTE);
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(81)
                .expect("later jurisdiction priority should validate"),
        },
    )
    .expect("later jurisdiction revision should validate")
    .commit(&mut state)
    .expect("later jurisdiction revision should commit");

    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist before corruption")
        .clone();
    assert_eq!(response.jurisdiction_version(), 1);
    let mut corrupted = police_response_wire(&response);
    corrupted.state.jurisdiction_version = 2;

    let error = restore_save(
        &registry,
        replace_serialized_police_response(
            build_save(&registry, &state)
                .expect("valid historical response should save before corruption"),
            &response,
            &corrupted,
        ),
    )
    .expect_err(
        "a jurisdiction revision created after dispatch cannot be forged into its snapshot",
    );
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPoliceResponse {
                response: invalid
            }
        ) if invalid == response_id
    ));
}

#[test]
fn later_higher_priority_jurisdiction_does_not_rewrite_historical_dispatch_routing() {
    let (registry, mut state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("operation should dispatch a response");
    let original_authority = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .authority();

    state.advance_clock(SimDuration::ONE_MINUTE);
    let later_authority = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Later Priority Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("later authority should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: later_authority,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(100)
                .expect("higher later priority should validate"),
        },
    )
    .expect("later jurisdiction should validate")
    .commit(&mut state)
    .expect("later jurisdiction should commit");
    assert_eq!(
        crate::legal::jurisdiction_system::resolve_police_response_authority(&state, neighborhood),
        Some(later_authority)
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &state)
            .expect("later routing changes must not invalidate historical responses"),
    )
    .expect("historical response should restore after a later routing change");
    assert_eq!(
        restored
            .legal()
            .get_police_response(response_id)
            .expect("restored historical response should persist")
            .authority(),
        original_authority
    );
}

#[test]
fn restore_rejects_police_response_patrol_revision_created_after_dispatch() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(20).expect("fixture patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("initial patrol should validate")
    .commit(&mut state)
    .expect("initial patrol should commit");
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("operation should dispatch a response");

    state.advance_clock(SimDuration::ONE_MINUTE);
    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![
            PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(80).expect("future patrol presence should validate"),
            )
            .expect("future patrol window should validate"),
        ],
    )
    .expect("later patrol revision should validate")
    .commit(&mut state)
    .expect("later patrol revision should commit");

    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist before corruption")
        .clone();
    assert_eq!(
        response.patrol(),
        Some(PoliceResponsePatrolSnapshot::new(deployment, 1))
    );
    let mut corrupted = police_response_wire(&response);
    corrupted.state.patrol = Some(PoliceResponsePatrolSnapshot::new(deployment, 2));

    let error = restore_save(
        &registry,
        replace_serialized_police_response(
            build_save(&registry, &state)
                .expect("valid historical response should save before corruption"),
            &response,
            &corrupted,
        ),
    )
    .expect_err("a patrol revision created after dispatch cannot be forged into its snapshot");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPoliceResponse {
                response: invalid
            }
        ) if invalid == response_id
    ));
}

#[test]
fn restore_rejects_police_response_presence_that_disagrees_with_frozen_patrol_revision() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(20).expect("fixture patrol presence should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("initial patrol should validate")
    .commit(&mut state)
    .expect("initial patrol should commit");
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("operation should dispatch a response");
    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist before corruption")
        .clone();
    assert_eq!(
        response.patrol(),
        Some(PoliceResponsePatrolSnapshot::new(deployment, 1))
    );
    assert_eq!(response.response_presence().value(), 20);
    let mut corrupted = police_response_wire(&response);
    corrupted.state.response_presence =
        Rating::try_new(80).expect("corrupted but in-range response presence should serialize");

    let error = restore_save(
        &registry,
        replace_serialized_police_response(
            build_save(&registry, &state)
                .expect("valid response should save before presence corruption"),
            &response,
            &corrupted,
        ),
    )
    .expect_err("response presence must be the value frozen from the referenced patrol revision");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPoliceResponse {
                response: invalid
            }
        ) if invalid == response_id
    ));
}

#[test]
fn restore_rejects_arrived_police_response_version_without_a_second_mutation() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let response_id = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("jurisdictional operation should dispatch a response");
    loop {
        let outcome = run_tick(&registry, &mut state);
        if outcome.arrived_police_responses.contains(&response_id) {
            break;
        }
        assert!(
            state
                .legal()
                .get_police_response(response_id)
                .expect("response should persist while dispatched")
                .status()
                == PoliceResponseStatus::Dispatched,
            "response must remain dispatched until its single arrival transition"
        );
    }
    let response = state
        .legal()
        .get_police_response(response_id)
        .expect("arrived response should persist")
        .clone();
    assert_eq!(response.status(), PoliceResponseStatus::Arrived);
    assert_eq!(response.version(), 2);
    let mut corrupted = police_response_wire(&response);
    corrupted.state.version = 3;

    let error = restore_save(
        &registry,
        replace_serialized_police_response(
            build_save(&registry, &state)
                .expect("valid arrived response should save before version corruption"),
            &response,
            &corrupted,
        ),
    )
    .expect_err("police responses have no post-arrival mutation that can create version 3");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPoliceResponse {
                response: invalid
            }
        ) if invalid == response_id
    ));
}

#[test]
fn patrol_established_at_resolution_does_not_rewrite_elapsed_operation_pressure() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(70).expect("fixture patrol rating should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should resolve against the patrol history that actually elapsed");
    assert_eq!(
        plan.outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(90),
        "a patrol established at the resolution instant must not backdate its presence across the operation"
    );

    // A same-instant revision also starts after the half-open elapsed window [start, resolution).
    // It therefore cannot stale or rewrite a plan whose historical police context is already fixed.
    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![
            PatrolWindow::try_new(
                DayMinute::try_new(600).expect("fixture minute should validate"),
                120,
                Rating::try_new(80).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate"),
        ],
    )
    .expect("patrol revision should validate")
    .commit(&mut state)
    .expect("patrol revision should commit");
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("future patrol state must not stale a fixed historical resolution window")
        .commit(&mut state)
        .expect("historically stable resolution should commit");
    validate_state(&state).expect("patrol-aware operation resolution should remain valid");
    validate_invariants(&state);
}

#[test]
fn operation_resolution_preserves_mid_execution_patrol_revision_history() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(70).expect("fixture patrol rating should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("initial patrol deployment should validate")
    .commit(&mut state)
    .expect("initial patrol deployment should commit");
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.now, SimTime::from_minutes(1));
    assert_eq!(start.started_operations, vec![operation]);

    state.advance_clock(SimDuration::from_minutes(20));
    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![
            PatrolWindow::try_new(
                DayMinute::try_new(600).expect("fixture minute should validate"),
                120,
                Rating::try_new(80).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate"),
        ],
    )
    .expect("mid-operation patrol revision should validate")
    .commit(&mut state)
    .expect("mid-operation patrol revision should commit");
    state.advance_clock(SimDuration::from_minutes(25));

    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should resolve across both historical patrol revisions");
    assert_eq!(
        plan.outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(31),
        "20 minutes at presence 70 followed by 25 minutes in an explicit zero-presence gap must average to 31"
    );
    assert_eq!(
        plan.outcome
            .exposure
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(31)
    );

    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![
            PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(90).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate"),
        ],
    )
    .expect("boundary patrol revision should validate")
    .commit(&mut state)
    .expect("boundary patrol revision should commit");
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("boundary patrol change must not stale immutable elapsed history")
        .commit(&mut state)
        .expect("historical patrol-aware resolution should commit");
    validate_state(&state).expect("patrol-aware operation resolution should remain valid");
    validate_invariants(&state);
}

#[test]
fn operation_resolution_uses_time_weighted_patrol_presence_across_execution_window() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(45).expect("fixture patrol minute should validate"),
                    60,
                    Rating::try_new(90).expect("fixture patrol rating should validate"),
                )
                .expect("fixture patrol window should validate"),
            ],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");

    let start = run_tick(&registry, &mut state);
    assert_eq!(start.now, SimTime::from_minutes(1));
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));

    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should resolve across its whole execution window");
    assert_eq!(
        plan.outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(2)
    );
    assert_eq!(
        plan.outcome
            .exposure
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(2)
    );
    assert!(
        !plan
            .narrative
            .summary
            .contains("High local police presence materially increased execution pressure.")
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("time-weighted patrol plan should validate")
        .commit(&mut state)
        .expect("time-weighted patrol resolution should commit");
    validate_state(&state).expect("time-weighted patrol state should validate");
    validate_invariants(&state);
}
