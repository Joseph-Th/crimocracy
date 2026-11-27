//! Restore must reconstruct operation timing and attribution, not merely accept
//! mutually consistent outcome arithmetic or an existing participant ID.

use super::*;
use crate::core::invariants::StateValidationError;
use crate::core::persistence::LoadError;

fn replace_wire<T: Serialize>(
    envelope: &SaveEnvelope,
    original: T,
    replacement: T,
) -> SaveEnvelope {
    let original = bincode::serialize(&original).unwrap();
    let replacement = bincode::serialize(&replacement).unwrap();
    assert_eq!(original.len(), replacement.len());
    let mut bytes = bincode::serialize(envelope).unwrap();
    let offsets: Vec<_> = bytes
        .windows(original.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == original).then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 1, "corrupted wire field must be unambiguous");
    let start = offsets[0];
    bytes[start..start + original.len()].copy_from_slice(&replacement);
    bincode::deserialize(&bytes).unwrap()
}

#[test]
fn restore_rejects_false_operation_schedule_before_completion() {
    let (registry, mut state, _, _, operation) =
        make_exposed_operation_fixture(OperationKind::Burglary, false, Vec::new());
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let record = state.operations().get_operation(operation).unwrap();
    let status = record.status();
    let start = record.started_at();
    let due = record.resolution_due_at();
    let entry = record.entry_at();
    let earlier_due = Some(SimTime::from_minutes(due.unwrap().as_minutes() - 1));
    let later_entry = Some(entry.unwrap().checked_add(SimDuration::ONE_MINUTE).unwrap());
    let envelope = build_save(&registry, &state).unwrap();
    for (replacement_due, replacement_entry) in [(earlier_due, entry), (due, later_entry)] {
        let corrupt = replace_wire(
            &envelope,
            (status, start, due, entry),
            (status, start, replacement_due, replacement_entry),
        );
        assert_eq!(
            restore_save(&registry, corrupt).err(),
            Some(LoadError::InvalidState(
                StateValidationError::InvalidOperationDefinition { operation }
            ))
        );
    }
    let mut restored = restore_save(&registry, envelope).unwrap();
    run_until_operation_resolved(&registry, &mut state, operation);
    run_until_operation_resolved(&registry, &mut restored, operation);
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn restore_reconstructs_shifted_operation_milestones_from_continue_decision() {
    let (registry, mut state, _, _, operation) = make_exposed_operation_fixture(
        OperationKind::Burglary,
        true,
        vec![OperationContingency::RequestDecisionOnPoliceArrival],
    );
    let owner = designate_operation_owner_as_player(&mut state, operation);
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let paused_at = run_until_police_arrival_decision(&registry, &mut state, operation);
    let record = state.operations().get_operation(operation).unwrap();
    let old_due = record.resolution_due_at().unwrap();
    let old_entry = record.entry_at().unwrap();
    assert!(paused_at < old_entry, "fixture must pause before entry");
    let decision = state.decisions().pending_for_operation(operation).unwrap();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    for branch in [&mut state, &mut restored] {
        for _ in 0..3 {
            run_tick(&registry, branch);
        }
        validate_resolve_decision(
            &registry,
            branch,
            decision,
            owner,
            DecisionResponse::Continue,
        )
        .unwrap()
        .commit(branch)
        .unwrap();
        let record = branch.operations().get_operation(operation).unwrap();
        assert_eq!(record.status(), OperationStatus::InProgress);
        assert_eq!(
            record.resolution_due_at(),
            old_due.checked_add(SimDuration::from_minutes(3))
        );
        assert_eq!(
            record.entry_at(),
            old_entry.checked_add(SimDuration::from_minutes(3))
        );
        let status = record.status();
        let start = record.started_at();
        let due = record.resolution_due_at();
        let entry = record.entry_at();
        let envelope = build_save(&registry, branch).unwrap();
        let corrupt = replace_wire(
            &envelope,
            (status, start, due, entry),
            (status, start, Some(old_due), Some(old_entry)),
        );
        assert_eq!(
            restore_save(&registry, corrupt).err(),
            Some(LoadError::InvalidState(
                StateValidationError::InvalidOperationDefinition { operation }
            ))
        );
        *branch = restore_save(&registry, envelope).unwrap();
        run_until_operation_resolved(&registry, branch, operation);
        validate_state_against_registry(&registry, branch).unwrap();
    }
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn restore_rejects_false_deadline_pressure_even_with_unchanged_margin() {
    let (registry, mut state, _, _, operation) = make_exposed_operation_fixture_with_constraints(
        OperationKind::Intimidation,
        false,
        Vec::new(),
        vec![OperationConstraint::CompleteBy(SimTime::from_minutes(11))],
    );
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    state.advance_clock(SimDuration::from_minutes(10));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(-5, 0),
    )
    .unwrap();
    validate_operation_resolution_plan(&registry, &state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let record = state.operations().get_operation(operation).unwrap();
    let resolution = record.resolution().unwrap();
    let factors = resolution.factors();
    assert_eq!(factors.time_pressure(), 15);
    let replacement = OperationResolutionFactors {
        time_pressure: 14,
        variance: -6,
        ..factors
    };
    assert_eq!(
        resolve_execution_margin(
            registry.get_operation(record.kind()).execution(),
            replacement
        ),
        resolution.execution_margin()
    );
    let envelope = build_save(&registry, &state).unwrap();
    assert_eq!(
        restore_save(&registry, replace_wire(&envelope, factors, replacement)).err(),
        Some(LoadError::InvalidState(
            StateValidationError::InvalidOperationDefinition { operation }
        ))
    );
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restore_save(&registry, envelope).unwrap()).unwrap()
    );
}

#[test]
fn restore_rejects_false_identified_participant_without_incident() {
    let (registry, mut state, _, _, operation) =
        make_exposed_operation_fixture(OperationKind::Burglary, false, Vec::new());
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let record = state.operations().get_operation(operation).unwrap();
    let due = record.resolution_due_at().unwrap();
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due.as_minutes() - state.now().as_minutes()).unwrap(),
    ));
    let variance = i8::try_from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .exposure_variance_limit(),
    )
    .unwrap();
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, variance),
    )
    .unwrap();
    validate_operation_resolution_plan(&registry, &state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let record = state.operations().get_operation(operation).unwrap();
    let exposure = record.resolution().unwrap().exposure();
    assert_eq!(exposure.level(), OperationExposureLevel::Identifying);
    assert!(exposure.investigation().is_none());
    assert_eq!(exposure.identified_character(), Some(record.leader()));
    let other = record
        .participant_ids()
        .find(|&participant| participant != record.leader())
        .unwrap();
    let prefix = (
        exposure.level(),
        exposure.score(),
        exposure.factors(),
        exposure.neighborhood(),
    );
    let envelope = build_save(&registry, &state).unwrap();
    // Nested tuples serialize in field order without additional framing.
    let corrupt = replace_wire(
        &envelope,
        (prefix, exposure.identified_character()),
        (prefix, Some(other)),
    );
    assert_eq!(
        restore_save(&registry, corrupt).err(),
        Some(LoadError::InvalidState(
            StateValidationError::InvalidOperationExposure { operation }
        ))
    );
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restore_save(&registry, envelope).unwrap()).unwrap()
    );
}
