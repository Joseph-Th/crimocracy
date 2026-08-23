//! Release-safe structural validation for the opportunities subsystem.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::legal::{EvidenceReliability, EvidenceStrength};
use crate::operations::OperationExposureLevel;
use crate::opportunities::OpportunityResolution;
use crate::reports::ReportKind;
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

pub(super) fn validate_opportunities(state: &AppState) -> Result<(), StateValidationError> {
    for opportunity in state.opportunities.opportunities() {
        let organization = state
            .world
            .get_organization(opportunity.organization())
            .ok_or(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            })?;
        let context = opportunity.context().operation();
        if organization.kind() != OrganizationKind::Criminal
            || context.targets().is_empty()
            || opportunity.source_information().is_empty()
            || opportunity.summary().trim().is_empty()
            || opportunity.discovered_at() > state.now()
            || opportunity.version() == 0
            || opportunity
                .valid_until()
                .is_some_and(|valid_until| valid_until <= opportunity.discovered_at())
        {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        let mut covered_targets = BTreeSet::new();
        for target in context.targets() {
            if !is_entity_present(state, *target) {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
        }
        for source in opportunity.source_information() {
            let information = state.intelligence.get_information(*source).ok_or(
                StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                },
            )?;
            if information.holder() != KnowledgeHolder::Organization(opportunity.organization())
                || information.recorded_at() > opportunity.discovered_at()
                || !context.targets().contains(&information.subject())
            {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
            covered_targets.insert(information.subject());
        }
        if covered_targets != *context.targets() {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        let report = state.reports.get_report(opportunity.report()).ok_or(
            StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            },
        )?;
        let mut expected_entities = context.targets().clone();
        expected_entities.insert(EntityRef::Organization(opportunity.organization()));
        let expected_sources: Vec<_> = opportunity.source_information().iter().copied().collect();
        if report.recipient() != opportunity.organization()
            || report.kind() != ReportKind::Opportunity
            || report.generated_at() != opportunity.discovered_at()
            || report.entries().len() != 1
            || !report.entries().first().is_some_and(|entry| {
                entry.attention == AttentionClass::Notable
                    && entry.summary == opportunity.summary()
                    && entry.sources == expected_sources
                    && entry.entities == expected_entities
                    && entry.decision.is_none()
            })
        {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        match opportunity.resolution() {
            None => {
                if opportunity.version() != 1
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| valid_until <= state.now())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Dismissed { at }) => {
                if opportunity.version() != 2
                    || at < opportunity.discovered_at()
                    || at > state.now()
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| at >= valid_until)
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Expired { at, report }) => {
                let expiry_report = state.reports.get_report(report).ok_or(
                    StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    },
                )?;
                if opportunity.version() != 2
                    || opportunity.valid_until() != Some(at)
                    || at > state.now()
                    || expiry_report.recipient() != opportunity.organization()
                    || expiry_report.kind() != ReportKind::Opportunity
                    || expiry_report.generated_at() < at
                    || expiry_report.generated_at() > state.now()
                    || expiry_report.entries().len() != 1
                    || !expiry_report.entries().first().is_some_and(|entry| {
                        entry.attention == AttentionClass::Notable
                            && entry.summary
                                == crate::opportunities::opportunity_system::expiry_report_summary(
                                    opportunity.summary(),
                                )
                            && entry.sources == expected_sources
                            && entry.entities == expected_entities
                            && entry.decision.is_none()
                    })
                    || state
                        .opportunities
                        .opportunity_for_report(report)
                        .map(|record| record.id())
                        != Some(opportunity.id())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Converted { at, operation }) => {
                let operation = state.operations.get_operation(operation).ok_or(
                    StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    },
                )?;
                let operation_targets: BTreeSet<_> = operation
                    .objective()
                    .referenced_entities()
                    .into_iter()
                    .collect();
                if opportunity.version() != 2
                    || at < opportunity.discovered_at()
                    || at > state.now()
                    || at > operation.scheduled_for()
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| at >= valid_until)
                    || operation.responsible_organization() != opportunity.organization()
                    || operation.kind() != context.operation_kind()
                // A converted operation acts against one of the discovered targets; its
                // objective carries exactly one referenced entity (see conversion matching).
                    || operation_targets.len() != 1
                    || !operation_targets
                        .iter()
                        .all(|target| context.targets().contains(target))
                    || state
                        .opportunities
                        .opportunity_for_operation(operation.id())
                        .map(|record| record.id())
                        != Some(opportunity.id())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_operation_exposure_links(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    if let Some(neighborhood) = exposure.neighborhood() {
        if state.world.get_neighborhood(neighborhood).is_none() {
            return Err(StateValidationError::InvalidOperationExposure {
                operation: operation.id(),
            });
        }
    }
    let participants: BTreeSet<_> = std::iter::once(operation.leader())
        .chain(operation.roles().values().copied())
        .collect();
    match exposure.level() {
        OperationExposureLevel::Identifying => {
            if !exposure
                .identified_character()
                .is_some_and(|character| participants.contains(&character))
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
        OperationExposureLevel::None
        | OperationExposureLevel::Trace
        | OperationExposureLevel::Witnessed => {
            if exposure.identified_character().is_some() {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
    }

    match exposure.investigation() {
        None => {
            if !exposure.evidence().is_empty() {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
        Some(investigation_id) => {
            if exposure.level() == OperationExposureLevel::None
                || exposure.neighborhood().is_none()
                || exposure.evidence().len() != 1
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
            let investigation = state.legal.get_investigation(investigation_id).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            let owner = state.world.get_organization(investigation.owner()).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            if !matches!(
                owner.kind(),
                OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
            ) || investigation.opened_at() != resolution.resolved_at()
                || !investigation
                    .subjects()
                    .contains(&EntityRef::Operation(operation.id()))
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
            if let Some(character) = exposure.identified_character() {
                if !investigation
                    .subjects()
                    .contains(&EntityRef::Character(character))
                {
                    return Err(StateValidationError::InvalidOperationExposure {
                        operation: operation.id(),
                    });
                }
            }
            let evidence_id = *exposure
                .evidence()
                .iter()
                .next()
                .expect("validated operation exposure contains one evidence record");
            let evidence = state.legal.get_evidence(evidence_id).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            let expected_subject = exposure
                .identified_character()
                .map(EntityRef::Character)
                .unwrap_or(EntityRef::Operation(operation.id()));
            let expected_strength = match exposure.level() {
                OperationExposureLevel::None => {
                    unreachable!("non-exposure cannot have legal evidence")
                }
                OperationExposureLevel::Trace => EvidenceStrength::Weak,
                OperationExposureLevel::Witnessed => EvidenceStrength::Corroborating,
                OperationExposureLevel::Identifying => EvidenceStrength::Strong,
            };
            let expected_reliability = match exposure.level() {
                OperationExposureLevel::None => {
                    unreachable!("non-exposure cannot have legal evidence")
                }
                OperationExposureLevel::Trace => EvidenceReliability::Questionable,
                OperationExposureLevel::Witnessed => EvidenceReliability::Credible,
                OperationExposureLevel::Identifying => EvidenceReliability::HighlyReliable,
            };
            if evidence.investigation() != investigation_id
                || evidence.custodian() != investigation.owner()
                || evidence.subject() != expected_subject
                || evidence.origin() != Some(EntityRef::Operation(operation.id()))
                || evidence.strength() != expected_strength
                || evidence.reliability() != expected_reliability
                || evidence.discovered_at() != resolution.resolved_at()
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
    }
    Ok(())
}
