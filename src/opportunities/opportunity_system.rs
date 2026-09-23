//! Validation, lifecycle transitions, and deterministic expiry for provenance-backed operation opportunities.

use crate::core::attention::AttentionClass;
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    IdExhaustionError, IdKind, InformationId, OperationId, OpportunityId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::intelligence::{InformationSignal, KnowledgeHolder, LegalPersonStatusSignal};
use crate::operations::operation_basis_knowledge::{
    source_information_is_usable_for_operation_basis, source_information_proves_operation_basis,
};
use crate::operations::operation_objective::pressureable_witness_targets_for_cases;
use crate::operations::operation_scheduling::resolve_operation_earliest_start;
use crate::operations::operation_system::is_actionable_opportunity_target;
use crate::operations::{OperationKind, OperationStatus};
use crate::opportunities::{
    OperationOpportunityContext, OperationOpportunityDraft, OpportunityRecord, OpportunityStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::OrganizationKind;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OpportunityError {
    #[error("opportunity summary must not be empty")]
    EmptySummary,
    #[error("opportunity organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("opportunity organization {0} is not a criminal organization")]
    InvalidOrganizationKind(OrganizationId),
    #[error("operation opportunity must reference at least one target entity")]
    MissingTargets,
    #[error("operation opportunity has no target currently actionable by {0:?}")]
    NoActionableTarget(OperationKind),
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
    #[error("opportunity target {0:?} has no usable direct source-information record")]
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
        "operation {operation} cannot begin until {earliest_start:?}, at or after opportunity validity deadline {valid_until:?}"
    )]
    OperationStartsAfterWindow {
        operation: OperationId,
        earliest_start: SimTime,
        valid_until: SimTime,
    },
    #[error(
        "operation {operation} schedule {scheduled_for:?} already passed before conversion at {now:?}"
    )]
    OperationSchedulePassed {
        operation: OperationId,
        scheduled_for: SimTime,
        now: SimTime,
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
    #[error(
        "opportunity {opportunity} changed after validation; expected version {expected}, found {found}"
    )]
    StaleOpportunity {
        opportunity: OpportunityId,
        expected: u32,
        found: u32,
    },
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {operation} is not authorized for opportunity conversion")]
    OperationNotAuthorized { operation: OperationId },
    #[error(
        "operation {operation} changed after opportunity conversion validation; expected version {expected}, found {found}"
    )]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "operation {operation} belongs to organization {operation_organization}, not opportunity organization {opportunity_organization}"
    )]
    OperationOrganizationMismatch {
        operation: OperationId,
        operation_organization: OrganizationId,
        opportunity_organization: OrganizationId,
    },
    #[error(
        "operation {operation} kind {operation_kind:?} does not match opportunity kind {opportunity_kind:?}"
    )]
    OperationKindMismatch {
        operation: OperationId,
        operation_kind: OperationKind,
        opportunity_kind: OperationKind,
    },
    #[error("operation {operation} must target exactly one entity covered by the opportunity")]
    OperationTargetsMismatch { operation: OperationId },
    #[error(
        "operation {operation} does not act on the same learned legal episode that created opportunity {opportunity}"
    )]
    OperationBasisMismatch {
        operation: OperationId,
        opportunity: OpportunityId,
    },
    #[error("operation {operation} is already linked to opportunity {opportunity}")]
    OperationAlreadyLinked {
        operation: OperationId,
        opportunity: OpportunityId,
    },
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

pub struct ValidatedOpportunityDiscovery<'registry> {
    draft: OperationOpportunityDraft,
    discovered_at: SimTime,
    report: ValidatedReport,
    registry: &'registry Registry,
}

impl ValidatedOpportunityDiscovery<'_> {
    pub fn commit(self, state: &mut AppState) -> Result<OpportunityId, OpportunityError> {
        state
            .ids
            .reserve_many(&[(IdKind::Report, 1), (IdKind::Opportunity, 1)])?;
        crate::core::time::ensure_time_current(state.now(), self.discovered_at).map_err(
            |(expected, found)| OpportunityError::StaleDiscoveryTime { expected, found },
        )?;
        validate_discovery_state(self.registry, state, &self.draft, self.discovered_at)?;
        let report = self
            .report
            .commit(state)
            .expect("opportunity report ID was preflighted before mutation");
        let id = state
            .ids
            .next_opportunity()
            .expect("opportunity ID was preflighted before mutation");
        state.opportunities.insert(OpportunityRecord {
            id,
            organization: self.draft.organization,
            context: OperationOpportunityContext {
                operation_kind: self.draft.operation_kind,
                targets: self.draft.targets,
            },
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

pub fn validate_discover_operation_opportunity<'registry>(
    registry: &'registry Registry,
    state: &AppState,
    draft: OperationOpportunityDraft,
) -> Result<ValidatedOpportunityDiscovery<'registry>, OpportunityError> {
    let discovered_at = state.now();
    let definition = registry.get_operation(draft.operation_kind);
    validate_discovery_state(registry, state, &draft, discovered_at)?;

    let mut entities = draft.targets.clone();
    entities.insert(EntityRef::Organization(draft.organization));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: draft.organization,
            kind: ReportKind::Opportunity,
            title: discovery_report_title(definition.display_name()),
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
        registry,
    })
}

/// Shared report-title and summary formats. The invariant validators re-derive these exact
/// strings to check persisted reports, so the producers and the validators must call the same
/// helpers rather than duplicating format strings.
pub(crate) fn discovery_report_title(display_name: &str) -> String {
    format!("{display_name} opportunity")
}

pub(crate) fn expiry_report_title(display_name: &str) -> String {
    format!("{display_name} opportunity expired")
}

pub(crate) fn expiry_report_summary(summary: &str) -> String {
    format!("Opportunity expired: {summary}")
}

pub(crate) fn dismissal_report_title(display_name: &str) -> String {
    format!("{display_name} opportunity dismissed")
}

pub(crate) fn dismissal_report_summary(summary: &str) -> String {
    format!("Opportunity dismissed: {summary}")
}

fn validate_discovery_state(
    registry: &Registry,
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
        if source_information_is_usable_for_operation_basis(
            registry,
            draft.operation_kind,
            information,
            discovered_at,
        ) {
            covered_targets.insert(information.subject());
        }
    }
    if let Some(uncovered) = draft
        .targets
        .iter()
        .find(|target| !covered_targets.contains(target))
    {
        return Err(OpportunityError::UncoveredTarget(*uncovered));
    }
    // Contextual entities need fresh organization-held coverage, but only the actual action
    // target must prove a sensitive legal basis. Hidden world state confirms whether a target is
    // actionable; typed information confirms that the organization legitimately knows why.
    let has_supported_actionable_target = draft.targets.iter().any(|target| {
        is_actionable_opportunity_target(
            registry,
            state,
            draft.organization,
            draft.operation_kind,
            *target,
        ) && draft.source_information.iter().any(|source| {
            state
                .intelligence
                .get_information(*source)
                .is_some_and(|information| {
                    information.subject() == *target
                        && source_information_proves_actionable_basis(
                            registry,
                            state,
                            draft.organization,
                            draft.operation_kind,
                            *target,
                            information,
                            discovered_at,
                        )
                })
        })
    });
    if !has_supported_actionable_target {
        return Err(OpportunityError::NoActionableTarget(draft.operation_kind));
    }
    if let Some(valid_until) = draft.valid_until
        && valid_until <= discovered_at
    {
        return Err(OpportunityError::InvalidValidityWindow {
            now: discovered_at,
            valid_until,
        });
    }
    if let Some(existing) = state.opportunities.find_open_operation(
        draft.organization,
        draft.operation_kind,
        &draft.targets,
    ) {
        return Err(OpportunityError::ExistingOpenOpportunity(existing.id()));
    }
    // Same-target work already has an open opportunity even when the target sets differ:
    // discovering `{A,B}` while `{A}` is open would leave a duplicate that outlives the
    // conversion of either one and expires with a redundant report.
    if let Some(existing) = state.opportunities.find_open_operation_overlapping(
        draft.organization,
        draft.operation_kind,
        &draft.targets,
    ) {
        return Err(OpportunityError::ExistingOpenOpportunity(existing.id()));
    }
    Ok(())
}

fn source_information_proves_actionable_basis(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    operation_kind: OperationKind,
    target: EntityRef,
    information: &crate::intelligence::InformationRecord,
    at: SimTime,
) -> bool {
    if !source_information_proves_operation_basis(
        registry,
        state,
        operation_kind,
        target,
        information,
        at,
    ) {
        return false;
    }
    match operation_kind {
        OperationKind::WitnessPressure => {
            let EntityRef::Character(character) = target else {
                return false;
            };
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
                investigation,
            })) = information.signal()
            else {
                return false;
            };
            let Some(case_witness) = state
                .legal
                .case_witness_for(*investigation, character)
                .map(|witness| witness.id())
            else {
                return false;
            };
            !pressureable_witness_targets_for_cases(
                state,
                organization,
                character,
                &BTreeSet::from([case_witness]),
            )
            .is_empty()
        }
        OperationKind::Extraction => {
            let EntityRef::Character(character) = target else {
                return false;
            };
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
                arrest,
            })) = information.signal()
            else {
                return false;
            };
            state
                .legal
                .active_arrest_for_character(character)
                .is_some_and(|active| active.id() == *arrest)
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::Surveillance
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Sabotage
        | OperationKind::Arson => true,
    }
}

pub struct ValidatedOpportunityDismissal {
    opportunity: OpportunityId,
    expected_version: u32,
    report: ValidatedReport,
}

impl ValidatedOpportunityDismissal {
    pub fn commit(self, state: &mut AppState) -> Result<ReportId, OpportunityError> {
        state.ids.reserve_many(&[(IdKind::Report, 1)])?;
        let record = validate_open_opportunity(state, self.opportunity)?;
        if record.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        ensure_version_can_advance(record.version(), "opportunity")?;
        validate_not_expired(state, record)?;
        let report = self
            .report
            .commit(state)
            .expect("dismissal report ID was preflighted before mutation");
        state
            .opportunities
            .dismiss(self.opportunity, state.now(), report);
        Ok(report)
    }
}

pub fn validate_dismiss_opportunity(
    registry: &Registry,
    state: &AppState,
    opportunity: OpportunityId,
) -> Result<ValidatedOpportunityDismissal, OpportunityError> {
    let record = validate_open_opportunity(state, opportunity)?;
    ensure_version_can_advance(record.version(), "opportunity")?;
    validate_not_expired(state, record)?;
    let definition = registry.get_operation(record.context().operation_kind());
    let mut entities = record.context().targets().clone();
    entities.insert(EntityRef::Organization(record.organization()));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.organization(),
            kind: ReportKind::Opportunity,
            title: dismissal_report_title(definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: dismissal_report_summary(record.summary()),
                sources: record.source_information().iter().copied().collect(),
                entities,
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedOpportunityDismissal {
        opportunity,
        expected_version: record.version(),
        report,
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
        ensure_version_can_advance(opportunity.version(), "opportunity")?;
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
    ensure_version_can_advance(opportunity_record.version(), "opportunity")?;
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
    if let Some(valid_until) = opportunity.valid_until()
        && state.now() >= valid_until
    {
        return Err(OpportunityError::OpportunityExpired {
            opportunity: opportunity.id(),
            valid_until,
        });
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
    if !operation_matches_opportunity_basis(state, opportunity, operation) {
        return Err(OpportunityError::OperationBasisMismatch {
            operation: operation.id(),
            opportunity: opportunity.id(),
        });
    }
    if state.now() > operation.scheduled_for() {
        return Err(OpportunityError::OperationSchedulePassed {
            operation: operation.id(),
            scheduled_for: operation.scheduled_for(),
            now: state.now(),
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
    let context = opportunity.context();
    if operation.kind() != context.operation_kind() {
        return Err(OpportunityError::OperationKindMismatch {
            operation: operation.id(),
            operation_kind: operation.kind(),
            opportunity_kind: context.operation_kind(),
        });
    }
    // Property-capable kinds can only authorize AcquireProperty objectives (enforced at
    // authorization), so a kind-matched conversion always carries the right objective.
    // An operation objective carries exactly one referenced entity, while a discovery may cover
    // several related targets. Conversion is coherent when the operation acts against one of the
    // opportunity's discovered targets; exact set equality would strand multi-target discoveries
    // in a permanent Open state.
    let operation_targets: BTreeSet<_> = operation
        .objective()
        .referenced_entities()
        .into_iter()
        .collect();
    if operation_targets.len() != 1
        || !operation_targets
            .iter()
            .all(|target| context.targets().contains(target))
    {
        return Err(OpportunityError::OperationTargetsMismatch {
            operation: operation.id(),
        });
    }
    // Discovery provenance remains owned by the opportunity record. Operation intelligence is a
    // different concern: it contains only facts the authored operation kind can use to improve
    // execution. Requiring those sets to overlap would strand legitimate openings whose basis is
    // not itself an execution modifier, such as learned custody or witness status.
    // The opportunity window is meaningful: conversion may only bind work whose planned earliest
    // start is inside the window. Begin-time validation rechecks the converted opportunity, so a
    // later crew delay cannot carry the operation past this viability boundary after conversion.
    let earliest_start = resolve_operation_earliest_start(operation);
    if opportunity
        .valid_until()
        .is_some_and(|valid_until| earliest_start >= valid_until)
    {
        return Err(OpportunityError::OperationStartsAfterWindow {
            operation: operation.id(),
            earliest_start,
            valid_until: opportunity
                .valid_until()
                .expect("expiry-checked opportunity has a validity window"),
        });
    }
    Ok(())
}

/// Sensitive legal openings are episode-specific. Discovery provenance remains separate from
/// execution intelligence, but conversion must not repurpose knowledge of one arrest or witness
/// registration into an operation against a later unrelated episode involving the same person.
pub(crate) fn operation_matches_opportunity_basis(
    state: &AppState,
    opportunity: &OpportunityRecord,
    operation: &crate::operations::OperationRecord,
) -> bool {
    match operation.kind() {
        OperationKind::Extraction => {
            let Some(arrest) = operation.extraction_arrest() else {
                return false;
            };
            opportunity.source_information().iter().any(|information| {
                state
                    .intelligence
                    .get_information(*information)
                    .is_some_and(|record| {
                        matches!(
                            record.signal(),
                            Some(InformationSignal::LegalPersonStatus(
                                LegalPersonStatusSignal::Detained {
                                    arrest: learned_arrest
                                }
                            )) if *learned_arrest == arrest
                        )
                    })
            })
        }
        OperationKind::WitnessPressure => {
            let pinned_investigations: BTreeSet<_> = operation
                .witness_pressure_cases()
                .iter()
                .filter_map(|case_witness| {
                    state
                        .legal
                        .get_case_witness(*case_witness)
                        .map(|witness| witness.investigation())
                })
                .collect();
            !pinned_investigations.is_empty()
                && opportunity.source_information().iter().any(|information| {
                    state
                        .intelligence
                        .get_information(*information)
                        .is_some_and(|record| {
                            matches!(
                                record.signal(),
                                Some(InformationSignal::LegalPersonStatus(
                                    LegalPersonStatusSignal::CaseWitness { investigation }
                                )) if pinned_investigations.contains(investigation)
                            )
                        })
                })
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::Surveillance
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Sabotage
        | OperationKind::Arson => true,
    }
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
        crate::core::time::ensure_time_current(state.now(), self.expected_now)
            .map_err(|(expected, found)| OpportunityError::StaleExpiryTime { expected, found })?;
        let opportunity = validate_open_opportunity(state, self.opportunity)?;
        if opportunity.version() != self.expected_version {
            return Err(OpportunityError::StaleOpportunity {
                opportunity: self.opportunity,
                expected: self.expected_version,
                found: opportunity.version(),
            });
        }
        ensure_version_can_advance(opportunity.version(), "opportunity")?;
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
    ensure_version_can_advance(record.version(), "opportunity")?;
    let valid_until = validate_expiry_due(state, record)?;
    let definition = registry.get_operation(record.context().operation_kind());
    let mut entities = record.context().targets().clone();
    entities.insert(EntityRef::Organization(record.organization()));
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.organization(),
            kind: ReportKind::Opportunity,
            title: expiry_report_title(definition.display_name()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: expiry_report_summary(record.summary()),
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
) -> Result<Vec<OpportunityId>, OpportunityError> {
    let due = state.opportunities.find_due_expiring(state.now());
    let mut validated = Vec::with_capacity(due.len());
    for opportunity in due {
        validated.push((
            opportunity,
            validate_expire_opportunity(registry, state, opportunity)?,
        ));
    }
    // Every due expiry writes one lifecycle report before the opportunity record changes.
    // Reserve the entire batch before the first report is persisted so allocator exhaustion
    // cannot expire only a prefix of the same-minute opportunity set.
    let report_budget = vec![(IdKind::Report, 1); validated.len()];
    state.ids.reserve_many(&report_budget)?;
    let mut expired = Vec::with_capacity(validated.len());
    for (opportunity, transaction) in validated {
        transaction
            .commit(state)
            .expect("preflighted opportunity expiry must remain valid within one batch");
        expired.push(opportunity);
    }
    Ok(expired)
}

#[cfg(test)]
mod tests;
