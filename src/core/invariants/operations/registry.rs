//! Registry-relative operation re-derivation and active-booking validation.

use super::*;

pub(super) fn validate_operations_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for operation in state.operations.operations() {
        validate_operation_against_registry(registry, state, operation)?;
    }
    validate_active_participant_bookings(registry, state)?;
    Ok(())
}

fn validate_active_participant_bookings(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let mut by_participant: BTreeMap<_, Vec<&OperationRecord>> = BTreeMap::new();
    for operation in state.operations.operations().filter(|operation| {
        !matches!(
            operation.status(),
            OperationStatus::Completed | OperationStatus::Aborted
        )
    }) {
        for participant in operation.participants() {
            by_participant
                .entry(participant)
                .or_default()
                .push(operation);
        }
    }
    for (participant, operations) in by_participant {
        if let Some((first, second)) =
            find_conflicting_participant_booking_pair(registry, state, &operations)
        {
            return Err(StateValidationError::ActiveOperationParticipantOverlap {
                participant,
                first: first.id(),
                second: second.id(),
            });
        }
    }
    Ok(())
}

fn find_conflicting_participant_booking_pair<'a>(
    registry: &Registry,
    state: &AppState,
    operations: &[&'a OperationRecord],
) -> Option<(&'a OperationRecord, &'a OperationRecord)> {
    operations
        .iter()
        .enumerate()
        .flat_map(|(index, first)| {
            operations[index + 1..]
                .iter()
                .map(move |second| (*first, *second))
        })
        .find(|(first, second)| active_booking_pair_conflicts(registry, state, first, second))
}

fn active_booking_pair_conflicts(
    registry: &Registry,
    state: &AppState,
    first: &OperationRecord,
    second: &OperationRecord,
) -> bool {
    let Some((first_start, first_end)) =
        resolve_operation_booking_window(registry, first, state.now())
    else {
        return false;
    };
    let Some((second_start, second_end)) =
        resolve_operation_booking_window(registry, second, state.now())
    else {
        return false;
    };
    if !(first_start < second_end && second_start < first_end) {
        return false;
    }
    // A live operation may legitimately grow into a later authorized booking when its begin was
    // delayed or a decision pause extended it. Canonical admission preserves the older
    // reservation and leaves the later operation queued. Prove that the pair was disjoint when
    // the later authorization happened instead of trusting status alone; two authorized records
    // never get this exception because their projected windows are immutable.
    let dynamic_overlap =
        matches!(
            (first.status(), second.status()),
            (OperationStatus::InProgress, OperationStatus::Authorized)
                | (OperationStatus::Authorized, OperationStatus::InProgress)
                | (
                    OperationStatus::AwaitingDecision,
                    OperationStatus::Authorized
                )
                | (
                    OperationStatus::Authorized,
                    OperationStatus::AwaitingDecision
                )
        ) && bookings_were_disjoint_at_later_authorization(registry, first, second);
    !dynamic_overlap
}

fn bookings_were_disjoint_at_later_authorization(
    registry: &Registry,
    first: &OperationRecord,
    second: &OperationRecord,
) -> bool {
    let (later, earlier) =
        if (first.authorized_at(), first.id()) > (second.authorized_at(), second.id()) {
            (first, second)
        } else {
            (second, first)
        };
    let at = later.authorized_at();
    let Some((later_start, later_end)) = resolve_operation_booking_window_at(registry, later, at)
    else {
        return false;
    };
    let Some((earlier_start, earlier_end)) =
        resolve_operation_booking_window_at(registry, earlier, at)
    else {
        return false;
    };
    !(later_start < earlier_end && earlier_start < later_end)
}

fn validate_operation_against_registry(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let definition = registry.get_operation(operation.kind());
    let execution = definition.execution();
    validate_authored_operation_plan(registry, state, operation, definition, execution)?;
    let Some(resolution) = operation.resolution() else {
        return Ok(());
    };
    let police_response_arrived =
        validate_authored_operation_resolution(registry, state, operation, execution, resolution)?;
    validate_authored_after_action_summary(registry, state, operation, resolution)?;
    validate_authored_property_disposition(registry, state, operation, resolution)?;
    exposure::validate_authored_operation_exposure(
        state,
        operation,
        execution,
        resolution,
        police_response_arrived,
    )
}

fn validate_authored_after_action_summary(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let expected = render_persisted_after_action_summary(registry, state, operation, resolution)
        .ok_or(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        })?;
    let information = state
        .intelligence
        .get_information(resolution.after_action_information())
        .ok_or(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        })?;
    let report = state
        .reports
        .get_report(resolution.after_action_report())
        .ok_or(StateValidationError::InvalidOperationAfterActionReport {
            operation: operation.id(),
        })?;
    if information.summary() != expected
        || report.entries().len() != 1
        || report.entries()[0].summary != expected
    {
        return Err(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_authored_operation_plan(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    definition: &OperationDefinition,
    execution: &OperationExecutionDefinition,
) -> Result<(), StateValidationError> {
    let has_police_entry_contingency = operation
        .contingencies()
        .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry);
    let police_response_matches_authorship = operation.police_response().is_none_or(|response| {
        state
            .legal
            .get_police_response(response)
            .is_some_and(|response| {
                let delay =
                    resolve_police_arrival_delay(execution, response.response_presence().value());
                response.alert_score() >= execution.police_dispatch_threshold()
                    && response
                        .dispatched_at()
                        .checked_add(crate::core::time::SimDuration::from_minutes(delay))
                        == Some(response.arrival_due_at())
            })
    });
    let deadline_window_is_valid = resolve_deadline_without_execution_window(
        execution,
        operation
            .started_at()
            .unwrap_or_else(|| resolve_operation_earliest_start(operation)),
        operation.constraints(),
    )
    .is_none();
    let authored_window_is_representable = operation
        .started_at()
        .unwrap_or_else(|| resolve_operation_earliest_start(operation))
        .as_minutes()
        .checked_add(u64::from(execution.duration().as_minutes()))
        .is_some();
    let before_start_deadline_abort_is_valid = operation.abort_record().is_none_or(|abort| {
        if abort.phase() != OperationAbortPhase::BeforeStart
            || abort.cause() != OperationAbortCause::DeadlineMissed
        {
            return true;
        }
        operation.completion_deadline().is_some_and(|deadline| {
            abort.aborted_at() >= deadline
                || resolve_deadline_without_execution_window(
                    execution,
                    abort.aborted_at(),
                    operation.constraints(),
                )
                .is_some()
        })
    });
    let business_target_is_valid = authored_business_target_is_valid(state, operation, execution);
    let planning_at = resolve_operation_earliest_start(operation);
    let max_intelligence_age = u64::from(execution.max_intelligence_age().as_minutes());
    let required_intelligence_is_usable = operation.constraints().iter().all(|constraint| {
        let OperationConstraint::RequireIntelligenceTopic(topic) = constraint else {
            return true;
        };
        operation.intelligence().iter().any(|information| {
            state
                .intelligence
                .get_information(*information)
                .is_some_and(|record| {
                    record.topic() == *topic
                        && resolve_information_score(
                            registry.information_quality(),
                            record,
                            planning_at,
                            max_intelligence_age,
                        ) > 0
                })
        })
    });
    let objective_target_is_participant = character_objective_target(operation.objective())
        .is_some_and(|target| operation.participants().contains(&target));
    let legal_basis_is_valid = match (operation.kind(), operation.objective()) {
        (
            OperationKind::WitnessPressure,
            OperationObjective::Frighten {
                target: EntityRef::Character(character),
            },
        ) => {
            !operation.witness_pressure_cases().is_empty()
                && operation
                    .witness_pressure_cases()
                    .iter()
                    .all(|case_witness| {
                        organization_knew_witness_case_at(
                            registry,
                            state,
                            operation.responsible_organization(),
                            *character,
                            *case_witness,
                            operation.authorized_at(),
                        )
                    })
        }
        (OperationKind::Extraction, OperationObjective::FreeDetainee { target }) => {
            operation.extraction_arrest().is_some_and(|arrest| {
                organization_knew_detention_at(
                    registry,
                    state,
                    operation.responsible_organization(),
                    *target,
                    arrest,
                    operation.authorized_at(),
                )
            })
        }
        _ => true,
    };
    let responsible_organization_is_criminal = state
        .world
        .get_organization(operation.responsible_organization())
        .is_some_and(|organization| {
            organization.kind() == crate::world::OrganizationKind::Criminal
        });
    if !definition
        .supported_approaches()
        .contains(&operation.approach())
        || definition
            .required_roles()
            .iter()
            .any(|role| !operation.roles().contains_key(role))
        || operation
            .roles()
            .keys()
            .any(|role| execution.capability_for_role(*role).is_none())
        || operation.intelligence().iter().any(|information| {
            state
                .intelligence
                .get_information(*information)
                .is_none_or(|record| {
                    !execution
                        .relevant_intelligence_topics()
                        .contains(&record.topic())
                })
        })
        || (has_police_entry_contingency && execution.operation_entry_offset().is_none())
        || (execution.operation_entry_offset().is_none() && operation.entry_at().is_some())
        || (operation.started_at().is_some()
            && execution.operation_entry_offset().is_some()
            && operation.entry_at().is_none())
        || !authored_window_is_representable
        || !deadline_window_is_valid
        || !before_start_deadline_abort_is_valid
        || !police_response_matches_authorship
        || !business_target_is_valid
        || !required_intelligence_is_usable
        || objective_target_is_participant
        || !legal_basis_is_valid
        || !responsible_organization_is_criminal
    {
        return Err(invalid_operation_definition(operation));
    }
    Ok(())
}

fn authored_business_target_is_valid(
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
) -> bool {
    let Some(ownership) = operation.kind().business_target_ownership() else {
        return execution.business_target().is_none();
    };
    let Some(business) = operation.objective().business_target() else {
        return false;
    };
    let Some(requirement) = execution.business_target() else {
        return false;
    };
    let Some(record) = state.world.get_business(business) else {
        return false;
    };
    if !requirement
        .required_functions()
        .iter()
        .all(|function| record.has_function(*function))
    {
        return false;
    }
    let (could_be_owned, definitely_owned) = state.world.business_owner_evidence_at(
        business,
        crate::world::BusinessOwner::Organization(operation.responsible_organization()),
        operation.authorized_at(),
    );
    match ownership {
        // Same-minute transfer ordering is not persisted. For a foreign target it is enough that
        // sponsor ownership was not certain for the whole timestamp; for a sponsor-hosted venue,
        // sponsor ownership must have been possible at some point in that timestamp.
        OperationBusinessTargetOwnership::Foreign => !definitely_owned,
        OperationBusinessTargetOwnership::SponsorOwned => could_be_owned,
    }
}

fn validate_authored_operation_resolution(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<bool, StateValidationError> {
    let factors = resolution.factors();
    let expected_margin = resolve_execution_margin(execution, factors);
    let base_expected_outcome = resolve_objective_outcome(execution, expected_margin);
    validate_resolution_objective_context(state, operation, resolution, base_expected_outcome)?;
    let expected_outcome =
        effective_objective_outcome(base_expected_outcome, resolution.objective_blocker());
    if matches!(
        operation.objective(),
        OperationObjective::FreeDetainee { .. }
    ) && expected_outcome != OperationObjectiveOutcome::Failed
    {
        let released_at = resolution
            .extraction_arrest()
            .and_then(|arrest| state.legal.get_arrest(arrest))
            .and_then(|arrest| arrest.released_at());
        if released_at != Some(resolution.resolved_at()) {
            return Err(invalid_operation_definition(operation));
        }
    }
    let (
        expected_intelligence_quality,
        expected_intelligence_adjustment,
        expected_intelligence_topics_covered,
        expected_intelligence_topics_relevant,
    ) = resolve_intelligence_factors(registry, state, operation.id());
    let expected_police_response_arrived =
        has_police_response_arrived_by(state, operation, resolution.resolved_at());
    let expected_property_proceeds =
        resolve_property_proceeds(registry, state, operation, expected_outcome)
            .map_err(|_| invalid_operation_definition(operation))?;
    let expected_cash_proceeds =
        resolve_cash_proceeds(registry, state, operation, expected_outcome).map_err(|_| {
            StateValidationError::InvalidOperationCashProceeds {
                operation: operation.id(),
            }
        })?;
    // An empty-handed take persists `Partial` while its proceeds re-derive from the
    // pre-downgrade tactical outcome. Mirror the planning rule exactly: the downgrade
    // consumes the proceeds derived above, and the comparisons below keep checking those
    // proceeds rather than a partial-basis re-derivation that would contradict them.
    let expected_outcome = downgrade_empty_take_outcome(
        execution,
        expected_outcome,
        expected_property_proceeds.proceeds.as_ref(),
        expected_cash_proceeds.proceeds.as_ref(),
    );
    if factors.variance().unsigned_abs() > execution.variance_limit()
        || factors.time_pressure() > execution.max_time_pressure()
        || factors.approach_adjustment()
            != execution
                .approach_difficulty_adjustment(operation.approach())
                .expect("validated operation approach must have an execution adjustment")
        || factors.business_fear_adjustment().unsigned_abs()
            > registry
                .reputation()
                .intimidation_business_fear_max_adjustment()
        || (operation.kind() != OperationKind::Intimidation
            && factors.business_fear_adjustment() != 0)
        || factors.intelligence_quality() != expected_intelligence_quality
        || factors.intelligence_adjustment() != expected_intelligence_adjustment
        || factors.intelligence_topics_covered() != expected_intelligence_topics_covered
        || factors.intelligence_topics_relevant() != expected_intelligence_topics_relevant
        || factors.intelligence_topics_covered() > factors.intelligence_topics_relevant()
        || factors.police_response_arrived() != expected_police_response_arrived
        || resolution.execution_margin() != expected_margin
        || resolution.objective_outcome() != expected_outcome
        || resolution.property_proceeds() != expected_property_proceeds.proceeds
    {
        return Err(invalid_operation_definition(operation));
    }
    if resolution.cash_proceeds() != expected_cash_proceeds.proceeds {
        return Err(StateValidationError::InvalidOperationCashProceeds {
            operation: operation.id(),
        });
    }
    Ok(expected_police_response_arrived)
}

fn validate_resolution_objective_context(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    base_expected_outcome: OperationObjectiveOutcome,
) -> Result<(), StateValidationError> {
    // Business ownership has append-only history, but different domains share minute-level
    // timestamps. An ownership transfer and an operation resolution with the same `SimTime` have
    // no persisted cross-domain ordering, so distinguish ownership that was possible at some
    // point during that timestamp from ownership that was true for every possible placement of
    // the resolution among the same-minute transfers.
    let sponsor_ownership_evidence =
        resolution_sponsor_ownership_evidence(state, operation, resolution);
    if base_expected_outcome != OperationObjectiveOutcome::Failed
        && ownership_mismatch_blocker_is_required(operation, sponsor_ownership_evidence)
        && resolution.objective_blocker()
            != Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    {
        return Err(invalid_operation_definition(operation));
    }
    validate_resolution_objective_blocker(
        state,
        operation,
        resolution,
        base_expected_outcome,
        sponsor_ownership_evidence,
    )?;
    validate_resolution_extraction_reference(state, operation, resolution)?;
    Ok(())
}

type BusinessOwnershipEvidence = (bool, bool);

fn resolution_sponsor_ownership_evidence(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Option<BusinessOwnershipEvidence> {
    operation.kind().business_target_ownership()?;
    let business = operation.objective().business_target()?;
    Some(state.world.business_owner_evidence_at(
        business,
        crate::world::BusinessOwner::Organization(operation.responsible_organization()),
        resolution.resolved_at(),
    ))
}

fn ownership_mismatch_blocker_is_required(
    operation: &OperationRecord,
    evidence: Option<BusinessOwnershipEvidence>,
) -> bool {
    evidence.is_some_and(|(could_be_owned, definitely_owned)| {
        match operation
            .kind()
            .business_target_ownership()
            .expect("persisted business-target operation kind must define ownership semantics")
        {
            OperationBusinessTargetOwnership::Foreign => definitely_owned,
            OperationBusinessTargetOwnership::SponsorOwned => !could_be_owned,
        }
    })
}

fn ownership_mismatch_blocker_is_possible(
    operation: &OperationRecord,
    evidence: Option<BusinessOwnershipEvidence>,
) -> bool {
    evidence.is_some_and(|(could_be_owned, definitely_owned)| {
        match operation
            .kind()
            .business_target_ownership()
            .expect("persisted business-target operation kind must define ownership semantics")
        {
            OperationBusinessTargetOwnership::Foreign => could_be_owned,
            OperationBusinessTargetOwnership::SponsorOwned => !definitely_owned,
        }
    })
}

fn validate_resolution_objective_blocker(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    base_expected_outcome: OperationObjectiveOutcome,
    sponsor_ownership_evidence: Option<BusinessOwnershipEvidence>,
) -> Result<(), StateValidationError> {
    let Some(blocker) = resolution.objective_blocker() else {
        return Ok(());
    };
    if base_expected_outcome == OperationObjectiveOutcome::Failed
        || !blocker_matches_objective(operation, blocker)
    {
        return Err(invalid_operation_definition(operation));
    }
    match blocker {
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
            if !ownership_mismatch_blocker_is_possible(operation, sponsor_ownership_evidence) {
                return Err(invalid_operation_definition(operation));
            }
        }
        OperationObjectiveBlocker::ExtractionCustodyEnded => {
            validate_extraction_custody_ended_blocker(state, operation, resolution)?;
        }
        // Business operating status and witness cooperation/case activity are mutable domains
        // without complete historical timelines. Their blocker is the persisted validated
        // resolution snapshot; objective compatibility above is the release-safe proof.
        OperationObjectiveBlocker::TargetEconomyInactive
        | OperationObjectiveBlocker::NoPressureableWitnessCase => {}
    }
    Ok(())
}

fn validate_extraction_custody_ended_blocker(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let OperationObjective::FreeDetainee { target } = operation.objective() else {
        return Err(invalid_operation_definition(operation));
    };
    let arrest = operation
        .extraction_arrest()
        .and_then(|id| state.legal.get_arrest(id))
        .ok_or_else(|| invalid_operation_definition(operation))?;
    if arrest.character() != *target
        || resolution.extraction_arrest().is_some()
        || arrest
            .released_at()
            .is_none_or(|released_at| released_at > resolution.resolved_at())
    {
        return Err(invalid_operation_definition(operation));
    }
    Ok(())
}

fn validate_resolution_extraction_reference(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let OperationObjective::FreeDetainee { target } = operation.objective() else {
        if resolution.extraction_arrest().is_some() {
            return Err(invalid_operation_definition(operation));
        }
        return Ok(());
    };
    let Some(arrest_id) = resolution.extraction_arrest() else {
        return Ok(());
    };
    if operation.extraction_arrest() != Some(arrest_id) {
        return Err(invalid_operation_definition(operation));
    }
    let arrest = state
        .legal
        .get_arrest(arrest_id)
        .ok_or_else(|| invalid_operation_definition(operation))?;
    if arrest.character() != *target
        || arrest.arrested_at() > resolution.resolved_at()
        || arrest
            .released_at()
            .is_some_and(|released_at| released_at < resolution.resolved_at())
    {
        return Err(invalid_operation_definition(operation));
    }
    Ok(())
}

fn validate_authored_property_disposition(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.property_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationPropertyDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.property_proceeds().ok_or_else(invalid)?;
    let expected_realized = resolve_property_liquidation_value(
        registry,
        state,
        operation.kind(),
        proceeds.estimated_value(),
        operation.id(),
        disposition.venue(),
    )
    .map_err(|_| invalid())?;
    if disposition.realized_value() != expected_realized {
        return Err(invalid());
    }
    Ok(())
}
