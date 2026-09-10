//! Registry-relative validation for synthesized report types.

use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::registry::Registry;
use std::collections::BTreeSet;

pub(super) fn validate_executive_briefs_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let mut generated = BTreeSet::new();
    for report in state
        .reports
        .reports()
        .filter(|report| report.kind() == crate::reports::ReportKind::ExecutiveBrief)
    {
        if report.title() != "Executive brief"
            || report.entries().is_empty()
            || !crate::reports::executive_brief::is_executive_brief_due(
                registry,
                report.generated_at(),
            )
            || !generated.insert((report.recipient(), report.generated_at()))
        {
            return Err(StateValidationError::InvalidExecutiveBrief {
                report: report.id(),
            });
        }
    }
    Ok(())
}
