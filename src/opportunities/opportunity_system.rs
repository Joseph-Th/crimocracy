//! Validation, lifecycle transitions, and deterministic expiry for provenance-backed operation opportunities.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    IdExhaustionError, IdKind, InformationId, OperationId, OpportunityId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::KnowledgeHolder;
use crate::operations::{OperationKind, OperationStatus};
use crate::opportunities::{
    OperationOpportunityContext, OperationOpportunityDraft, OpportunityContext, OpportunityRecord,
    OpportunityStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{Lifecycle, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OpportunityError {
    #[error("opportunity summary must not be empty")]
    EmptySummary,
    #[error("operation {operation} cites none of the information that discovered opportunity {opportunity}")]
    OperationLacksSourceIntelligence {
        operation: crate::core::id::OperationId,
        opportunity: OpportunityId,
    },
    #[error("opportunity organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("opportunity organization {0} is not active")]
    InactiveOrganization(OrganizationId),
    #[error("opportunity organization {0} is not a criminal organization")]
    InvalidOrganizationKind(OrganizationId),
    #[error("operation opportunity must reference at least one target entity")]
    MissingTargets,
    #[error("opportunity target entity {0:?} does not exist")]
    MissingTarget(EntityRef),
    #[error("operation opportunity must have at least one source-information record")]
    MissingSourceInformation,
    #[error("opportunity source-information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("information {information} is not held by opportunity organization {organization}")]
    InformationUnavailable {
        information: InformationId,
        organization: OrganizationId,
    },
    #[error("information {information} concerns {subject:?}, which is not an opportunity target")]
    InformationTargetMismatch {
        information: InformationId,
        subject: EntityRef,
    },
    #[error("opportunity target {0:?} has no direct source-information record")]
    UncoveredTarget(EntityRef),
    #[error(
        "opportunity validity deadline {valid_until:?} must be later than discovery time {now:?}"
    )]
    InvalidValidityWindow { now: SimTime, valid_until: SimTime },
    #[error("matching open opportunity {0} already exists")]
    ExistingOpenOpportunity(OpportunityId),
    #[error(
        "opportunity discovery was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleDiscoveryTime { expected: SimTime, found: SimTime },
    #[error("opportunity {0} does not exist")]
    MissingOpportunity(OpportunityId),
    #[error("opportunity {opportunity} is not open; current status is {status:?}")]
    OpportunityNotOpen {
        opportunity: OpportunityId,
        status: OpportunityStatus,
    },
    #[error("opportunity {opportunity} expired at {valid_until:?}")]
    OpportunityExpired {
        opportunity: OpportunityId,
        valid_until: SimTime,
    },
    #[error("opportunity {0} has no validity deadline and cannot expire automatically")]
    MissingValidityDeadline(OpportunityId),
    #[error(
        "operation {operation} is scheduled at or after opportunity validity deadline {valid_until:?}"
    )]
    OperationScheduledAfterWindow {
        operation: OperationId,
        valid_until: SimTime,
    },
    #[error(
        "opportunity {opportunity} does not expire until {valid_until:?}; current time is {now:?}"
    )]
    ExpiryNotDue {
        opportunity: OpportunityId,
        valid_until: SimTime,
        now: SimTime,
    },
    #[error(
        "opportunity expiry was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleExpiryTime { expected: SimTime, found: SimTime },
    #[error("opportunity {opportunity} changed after validation; expected version {expected}, found {found}")]
    StaleOpportunity {
        opportunity: OpportunityId,
        expected: u32,
        found: u32,
    },
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {operation} is not authorized for opportunity conversion")]
    OperationNotAuthorized { operation: OperationId },
    #[error("operation {operation} changed after opportunity conversion validation; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error("operation {operation} belongs to organization {operation_organization}, not opportunity organization {opportunity_organization}")]
    OperationOrganizationMismatch {
        operation: OperationId,
        operation_organization: OrganizationId,
        opportunity_organization: OrganizationId,
    },
    #[error("operation {operation} kind {operation_kind:?} does not match opportunity kind {opportunity_kind:?}")]
    OperationKindMismatch {
        operation: OperationId,
        operation_kind: OperationKind,
        opportunity_kind: OperationKind,
    },
    #[error("operation {operation} targets do not exactly match opportunity targets")]
    OperationTargetsMismatch { operation: OperationId },
    #[error("operation {operation} does not use the property-acquisition objective required by its opportunity")]
    OperationObjectiveMismatch { operation: OperationId },
    #[error("operation {operation} is already linked to opportunity {opportunity}")]
    OperationAlreadyLinked {
        operation: OperationId,
        opportunity: OpportunityId,
    },
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedOpportunityDiscovery {
    draft: OperationOpportunityDraft,
    discovered_at: SimTime,
    report: ValidatedReport,
}

impl ValidatedOpportunityDiscovery {
    pub fn commit(self, state: &mut AppState) -> Result<OpportunityId, OpportunityError> {
        state
            .ids
            .reserve_many(&[(IdKind::Report, 1), (IdKind::Opportunity, 1)])?;
        if state.now() != self.discovered_at {
            return Err(OpportunityError::StaleDiscoveryTime {
                expected: self.discovered_at,
                found: state.now(),
            });
        }
        validate_discovery_state(state, &self.draft, self.discovered_at)?;
        let report = self.report.commit(state)?;
        let id = state.ids.next_opportunity()?;
        state.opportunities.insert(OpportunityRecord {
            id,
            organization: self.draft.organization,
            context: OpportunityContext::Operation(OperationOpportunityContext {
                operation_kind: self.draft.operation_kind,
                targets: self.draft.targets,
            }),
            discovered_at: self.discovered_at,
            valid_until: self.draft.valid_until,
            source_information: self.draft.source_information,
            summary: self.draft.summary,
            report,
            resolution: None,
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_discover_operation_opportunity(
    registry: &Registry,
    state: &AppState,
    draft: OperationOpportunityDraft,
) -> Result<ValidatedOpportunityDiscovery, OpportunityError> {
    let discovered_at = state.now();
    let definition = registry.get_operation(draft.operation_kind);
    validate_discovery_state(state, &draft, discovered_at)?;

    let mut entities = draft.targets.clone();
    entities.insert(EntityRef::Organization(draft.organization));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: draft.organization,
            kind: ReportKind::Opportunity,
            title: format!("{} opportunity", definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: draft.summary.clone(),
                sources: draft.source_information.iter().copied().collect(),
                entities,
                decision: None,
            }],
        },
    )?;

    Ok(ValidatedOpportunityDiscovery {
        draft,
        discovered_at,
        report,
    })
}

fn validate_discovery_state(
    state: &AppState,
    draft: &OperationOpportunityDraft,
    discovered_at: SimTime,
) -> Result<(), OpportunityError> {
    if draft.summary.trim().is_empty() {
        return Err(OpportunityError::EmptySummary);
    }
    let organization = state
        .world
        .get_organization(draft.organization)
        .ok_or(OpportunityError::MissingOrganization(draft.organization))?;
    if organization.lifecycle() != Lifecycle::Active {
        return Err(OpportunityError::InactiveOrganization(draft.organization));
    }
    if organization.kind() != OrganizationKind::Criminal {
        return Err(OpportunityError::InvalidOrganizationKind(
            draft.organization,
        ));
    }
    if draft.targets.is_empty() {
        return Err(OpportunityError::MissingTargets);
    }
    for target in &draft.targets {
        if !is_entity_present(state, *target) {
            return Err(OpportunityError::MissingTarget(*target));
        }
    }
    if draft.source_information.is_empty() {
        return Err(OpportunityError::MissingSourceInformation);
    }
    let mut covered_targets = BTreeSet::new();
    for source in &draft.source_information {
        let information = state
            .intelligence
            .get_information(*source)
            .ok_or(OpportunityError::MissingInformation(*source))?;
        if information.holder() != KnowledgeHolder::Organization(draft.organization) {
            return Err(OpportunityError::InformationUnavailable {
                information: *source,
                organization: draft.organization,
            });
        }
        if !draft.targets.contains(&information.subject()) {
            return Err(OpportunityError::InformationTargetMismatch {
                information: *source,
                subject: information.subject(),
            });
        }
        covered_targets.insert(information.subject());
    }
    if let Some(uncovered) = draft
        .targets
        .iter()
        .find(|target| !covered_targets.contains(target))
    {
        return Err(OpportunityError::UncoveredTarget(*uncovered));
    }
    if let Some(valid_until) = draft.valid_until {
        if valid_until <= discovered_at {
            return Err(OpportunityError::InvalidValidityWindow {
                now: discovered_at,
                valid_until,
            });
        }
    }
    if let Some(existing) = state.opportunities.open_matching_operation(
        draft.organization,
        draft.operation_kind,
        &draft.targets,
    ) {
        return Err(OpportunityError::ExistingOpenOpportunity(existing.id()));
    }
    Ok(())
}

pub struct ValidatedOpportunityDismissal {
    opportunity: OpportunityId,
    expected_version: u32,
}

impl ValidatedOpportunityDismissal {
    pub fn commit(self, state: &mut AppState) -> Result<(), OpportunityError> {
        let record = validate_open_opportunity(state, self.opportunity)?;
        if record.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_not_expired(state, record)?;
        state.opportunities.dismiss(self.opportunity, state.now());
        Ok(())
    }
}

pub fn validate_dismiss_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<ValidatedOpportunityDismissal, OpportunityError> {
    let record = validate_open_opportunity(state, opportunity)?;
    validate_not_expired(state, record)?;
    Ok(ValidatedOpportunityDismissal {
        opportunity,
        expected_version: record.version(),
    })
}

pub struct ValidatedOpportunityConversion {
    opportunity: OpportunityId,
    expected_opportunity_version: u32,
    operation: OperationId,
    expected_operation_version: u32,
}

impl ValidatedOpportunityConversion {
    pub fn commit(self, state: &mut AppState) -> Result<(), OpportunityError> {
        let opportunity = validate_open_opportunity(state, self.opportunity)?;
        if opportunity.version() != self.expected_opportunity_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_opportunity_version,
                found: opportunity.version(),
            });
        }
        validate_not_expired(state, opportunity)?;
        let operation = state
            .operations
            .get_operation(self.operation)
            .ok_or(OpportunityError::MissingOperation(self.operation))?;
        if operation.version() != self.expected_operation_version {
            return Err(OpportunityError::StaleOperation {
                operation: self.operation,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        validate_conversion_match(state, opportunity, operation)?;
        state
            .opportunities
            .convert(self.opportunity, self.operation, state.now());
        Ok(())
    }
}

pub fn validate_convert_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
    operation: OperationId,
) -> Result<ValidatedOpportunityConversion, OpportunityError> {
    let opportunity_record = validate_open_opportunity(state, opportunity)?;
    validate_not_expired(state, opportunity_record)?;
    let operation_record = state
        .operations
        .get_operation(operation)
        .ok_or(OpportunityError::MissingOperation(operation))?;
    validate_conversion_match(state, opportunity_record, operation_record)?;
    Ok(ValidatedOpportunityConversion {
        opportunity,
        expected_opportunity_version: opportunity_record.version(),
        operation,
        expected_operation_version: operation_record.version(),
    })
}

fn validate_open_opportunity(
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<&OpportunityRecord, OpportunityError> {
    let record = state
        .opportunities
        .get_opportunity(opportunity)
        .ok_or(OpportunityError::MissingOpportunity(opportunity))?;
    if record.status() != OpportunityStatus::Open {
        return Err(OpportunityError::OpportunityNotOpen {
            opportunity,
            status: record.status(),
        });
    }
    Ok(record)
}

fn validate_not_expired(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), OpportunityError> {
    if let Some(valid_until) = opportunity.valid_until() {
        if state.now() >= valid_until {
            return Err(OpportunityError::OpportunityExpired {
                opportunity: opportunity.id(),
                valid_until,
            });
        }
    }
    Ok(())
}

fn validate_conversion_match(
    state: &AppState,
    opportunity: &OpportunityRecord,
    operation: &crate::operations::OperationRecord,
) -> Result<(), OpportunityError> {
    if operation.status() != OperationStatus::Authorized {
        return Err(OpportunityError::OperationNotAuthorized {
            operation: operation.id(),
        });
    }
    if let Some(existing) = state
        .opportunities
        .opportunity_for_operation(operation.id())
    {
        return Err(OpportunityError::OperationAlreadyLinked {
            operation: operation.id(),
            opportunity: existing.id(),
        });
    }
    if operation.responsible_organization() != opportunity.organization() {
        return Err(OpportunityError::OperationOrganizationMismatch {
            operation: operation.id(),
            operation_organization: operation.responsible_organization(),
            opportunity_organization: opportunity.organization(),
        });
    }
    let context = opportunity.context().operation();
    if operation.kind() != context.operation_kind() {
        return Err(OpportunityError::OperationKindMismatch {
            operation: operation.id(),
            operation_kind: operation.kind(),
            opportunity_kind: context.operation_kind(),
        });
    }
    // Property-capable kinds can only authorize AcquireProperty objectives (enforced at
    // authorization), so a kind-matched conversion always carries the right objective.
    let operation_targets: BTreeSet<_> = operation
        .objective()
        .referenced_entities()
        .into_iter()
        .collect();
    if operation_targets != *context.targets() {
        return Err(OpportunityError::OperationTargetsMismatch {
            operation: operation.id(),
        });
    }
    // Conversion preserves the discovery provenance chain: the operation must actually cite
    // information the opportunity was discovered through, not merely match its shape.
    if !operation
        .intelligence()
        .iter()
        .any(|information| opportunity.source_information().contains(information))
    {
        return Err(OpportunityError::OperationLacksSourceIntelligence {
            operation: operation.id(),
            opportunity: opportunity.id(),
        });
    }
    // The opportunity window is meaningful: converting must bind an operation that will actually
    // execute inside the window, not one scheduled after the opportunity has closed. Otherwise the
    // "valid until" deadline could be consumed by an operation that never runs while the situation
    // was live.
    if opportunity
        .valid_until()
        .is_some_and(|valid_until| operation.scheduled_for() >= valid_until)
    {
        return Err(OpportunityError::OperationScheduledAfterWindow {
            operation: operation.id(),
            valid_until: opportunity
                .valid_until()
                .expect("expiry-checked opportunity has a validity window"),
        });
    }
    Ok(())
}

struct ValidatedOpportunityExpiry {
    opportunity: OpportunityId,
    expected_version: u32,
    expected_now: SimTime,
    valid_until: SimTime,
    report: ValidatedReport,
}

impl ValidatedOpportunityExpiry {
    fn commit(self, state: &mut AppState) -> Result<ReportId, OpportunityError> {
        if state.now() != self.expected_now {
            return Err(OpportunityError::StaleExpiryTime {
                expected: self.expected_now,
                found: state.now(),
            });
        }
        let opportunity = validate_open_opportunity(state, self.opportunity)?;
        if opportunity.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: opportunity.version(),
            });
        }
        let valid_until = validate_expiry_due(state, opportunity)?;
        debug_assert_eq!(valid_until, self.valid_until);

        let report = self.report.commit(state)?;
        state
            .opportunities
            .expire(self.opportunity, self.valid_until, report);
        Ok(report)
    }
}

fn validate_expire_opportunity(
    registry: &Registry,
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<ValidatedOpportunityExpiry, OpportunityError> {
    let record = validate_open_opportunity(state, opportunity)?;
    let valid_until = validate_expiry_due(state, record)?;
    let definition = registry.get_operation(record.context().operation().operation_kind());
    let mut entities = record.context().operation().targets().clone();
    entities.insert(EntityRef::Organization(record.organization()));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.organization(),
            kind: ReportKind::Opportunity,
            title: format!("{} opportunity expired", definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!("Opportunity expired: {}", record.summary()),
                sources: record.source_information().iter().copied().collect(),
                entities,
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedOpportunityExpiry {
        opportunity,
        expected_version: record.version(),
        expected_now: state.now(),
        valid_until,
        report,
    })
}

fn validate_expiry_due(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<SimTime, OpportunityError> {
    let valid_until = opportunity
        .valid_until()
        .ok_or(OpportunityError::MissingValidityDeadline(opportunity.id()))?;
    if state.now() < valid_until {
        return Err(OpportunityError::ExpiryNotDue {
            opportunity: opportunity.id(),
            valid_until,
            now: state.now(),
        });
    }
    Ok(valid_until)
}

pub(crate) fn apply_opportunity_expiry(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<OpportunityId> {
    let due = state.opportunities.due_expiring_at_or_before(state.now());
    let mut expired = Vec::with_capacity(due.len());
    for opportunity in due {
        // Like autonomous recruitment and staffing, expiry is an autonomous pass: one record
        // that fails validation must not abort due work everywhere else in the same minute.
        let Ok(transaction) = validate_expire_opportunity(registry, state, opportunity) else {
            continue;
        };
        if transaction.commit(state).is_ok() {
            expired.push(opportunity);
        }
    }
    expired
}

#[cfg(test)]
mod tests;
