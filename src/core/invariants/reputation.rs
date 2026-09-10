//! Structural and registry-relative validation for sparse reputation state.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::registry::Registry;

pub(super) fn validate_reputations(state: &AppState) -> Result<(), StateValidationError> {
    if !state.reputation.has_consistent_indexes() {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "reputation",
        });
    }
    for record in state.reputation.records() {
        let entity = EntityRef::Organization(record.organization());
        if !is_entity_present(state, entity) {
            return Err(StateValidationError::MissingEntity {
                context: "reputation record",
                entity,
            });
        }
        for dimension in crate::reputation::ALL_REPUTATION_DIMENSIONS {
            if record.score(dimension) > 100 {
                return Err(StateValidationError::InvalidReputationScore {
                    organization: record.organization(),
                    audience: record.audience(),
                });
            }
            if record.changed_at(dimension) > state.now() {
                return Err(StateValidationError::InvalidReputationChronology {
                    organization: record.organization(),
                    audience: record.audience(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_reputations_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let baseline = registry.reputation().baseline();
    for record in state.reputation.records() {
        if crate::reputation::ALL_REPUTATION_DIMENSIONS
            .iter()
            .all(|dimension| record.score(*dimension) == baseline)
        {
            return Err(StateValidationError::NeutralReputationRecord {
                organization: record.organization(),
                audience: record.audience(),
            });
        }
    }
    Ok(())
}
