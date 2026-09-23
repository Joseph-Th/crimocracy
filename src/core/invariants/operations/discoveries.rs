//! Persisted operation-discovery provenance and semantic validation.

use super::*;

pub(super) fn validate_operation_discoveries(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    discovered_information: &mut BTreeSet<InformationId>,
    actual_signatures: &mut BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)>,
) -> Result<(), StateValidationError> {
    actual_signatures.clear();
    match operation.kind() {
        OperationKind::Surveillance | OperationKind::DocumentTheft => {
            let OperationObjective::GatherInformation { target } = operation.objective() else {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            };
            if (operation.kind() == OperationKind::Surveillance
                && !is_supported_surveillance_target(*target))
                || (operation.kind() == OperationKind::DocumentTheft
                    && !matches!(target, EntityRef::Business(_)))
            {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
            match resolution.objective_outcome() {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    if resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Failed
                    if !resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Achieved
                | OperationObjectiveOutcome::Partial
                | OperationObjectiveOutcome::Failed => {}
            }
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::WitnessPressure
        | OperationKind::GamblingEvent
        | OperationKind::Extraction
        | OperationKind::Sabotage
        | OperationKind::Arson => {
            if !resolution.discovered_information().is_empty() {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
        }
    }

    for information_id in resolution.discovered_information() {
        let information = state.intelligence.get_information(*information_id).ok_or(
            StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            },
        )?;
        if !discovered_information.insert(*information_id)
            || !actual_signatures.insert((
                information.topic(),
                information.subject(),
                information.signal().cloned(),
            ))
            || state
                .operations
                .operation_for_discovered_information(*information_id)
                .is_none_or(|source| source.id() != operation.id())
            || information.recorded_at() != resolution.resolved_at()
            || !is_valid_persisted_operation_information(operation, information)
        {
            return Err(StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            });
        }
    }
    // The resolution froze the semantic facts produced by information-acquisition work; the
    // persisted intelligence records must match that set exactly.
    if matches!(
        operation.kind(),
        OperationKind::Surveillance | OperationKind::DocumentTheft
    ) && resolution.discovery_signatures() != actual_signatures
    {
        return Err(StateValidationError::InvalidOperationDiscovery {
            operation: operation.id(),
        });
    }
    Ok(())
}
