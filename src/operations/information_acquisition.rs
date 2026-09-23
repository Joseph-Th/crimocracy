//! Dispatch for operation objectives that create durable intelligence.
//!
//! Pre-operation intelligence scoring lives in `operation_intelligence`. This module owns the
//! distinct post-resolution acquisition path so surveillance and stolen records share one
//! provenance, validation, and persistence contract without sharing their observation semantics.

use crate::core::entity::EntityRef;
use crate::core::id::{OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::intelligence::intelligence_system::ValidatedInformation;
use crate::intelligence::{InformationRecord, InformationSignal, InformationTopic};
use crate::operations::document_theft_integration::{
    DocumentTheftError, DocumentTheftIntelligencePlan, decide_document_theft_intelligence,
    document_theft_after_action_clause, is_valid_persisted_document_theft_information,
    persisted_document_theft_after_action_clause, validate_document_theft_information,
    validate_document_theft_plan_snapshot,
};
use crate::operations::surveillance_integration::{
    SurveillanceError, SurveillanceIntelligencePlan, decide_surveillance_intelligence,
    is_valid_persisted_surveillance_information, persisted_surveillance_after_action_clause,
    surveillance_after_action_clause, validate_surveillance_information,
    validate_surveillance_plan_snapshot,
};
use crate::operations::{OperationKind, OperationObjectiveOutcome, OperationRecord};
use crate::registry::Registry;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum InformationAcquisitionError {
    #[error(transparent)]
    Surveillance(#[from] SurveillanceError),
    #[error(transparent)]
    DocumentTheft(#[from] DocumentTheftError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OperationInformationPlan {
    Surveillance(SurveillanceIntelligencePlan),
    DocumentTheft(DocumentTheftIntelligencePlan),
}

impl OperationInformationPlan {
    pub(crate) fn discovery_signatures(
        &self,
    ) -> BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)> {
        match self {
            Self::Surveillance(plan) => plan.discovery_signatures(),
            Self::DocumentTheft(plan) => plan.discovery_signatures(),
        }
    }
}

pub(crate) fn decide_operation_information(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<Option<OperationInformationPlan>, InformationAcquisitionError> {
    match operation.kind() {
        OperationKind::Surveillance => {
            decide_surveillance_intelligence(registry, state, operation, outcome)
                .map(|plan| plan.map(OperationInformationPlan::Surveillance))
                .map_err(Into::into)
        }
        OperationKind::DocumentTheft => {
            decide_document_theft_intelligence(state, operation, outcome)
                .map(|plan| plan.map(OperationInformationPlan::DocumentTheft))
                .map_err(Into::into)
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
        | OperationKind::Arson => Ok(None),
    }
}

pub(crate) fn validate_operation_information_plan_snapshot(
    state: &AppState,
    plan: &OperationInformationPlan,
    off_window_patrol_presence_percent: u8,
) -> Result<(), InformationAcquisitionError> {
    match plan {
        OperationInformationPlan::Surveillance(plan) => {
            validate_surveillance_plan_snapshot(state, plan, off_window_patrol_presence_percent)
                .map_err(Into::into)
        }
        OperationInformationPlan::DocumentTheft(plan) => {
            validate_document_theft_plan_snapshot(state, plan).map_err(Into::into)
        }
    }
}

pub(crate) fn validate_operation_information(
    state: &AppState,
    organization: OrganizationId,
    source_operation: OperationId,
    plan: &OperationInformationPlan,
) -> Result<Vec<ValidatedInformation>, crate::intelligence::intelligence_system::IntelligenceError>
{
    match plan {
        OperationInformationPlan::Surveillance(plan) => {
            validate_surveillance_information(state, organization, source_operation, plan)
        }
        OperationInformationPlan::DocumentTheft(plan) => {
            validate_document_theft_information(state, organization, source_operation, plan)
        }
    }
}

pub(crate) fn operation_information_after_action_clause(
    plan: Option<&OperationInformationPlan>,
    outcome: OperationObjectiveOutcome,
) -> Option<String> {
    match plan {
        Some(OperationInformationPlan::Surveillance(plan)) => {
            surveillance_after_action_clause(Some(plan), outcome)
        }
        Some(OperationInformationPlan::DocumentTheft(plan)) => {
            document_theft_after_action_clause(Some(plan), outcome)
        }
        None => None,
    }
}

pub(crate) fn persisted_operation_information_after_action_clause(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<Option<String>, ()> {
    match operation.kind() {
        OperationKind::Surveillance => persisted_surveillance_after_action_clause(state, operation),
        OperationKind::DocumentTheft => {
            persisted_document_theft_after_action_clause(state, operation)
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
        | OperationKind::Arson => Ok(None),
    }
}

pub(crate) fn is_valid_persisted_operation_information(
    operation: &OperationRecord,
    information: &InformationRecord,
) -> bool {
    match operation.kind() {
        OperationKind::Surveillance => {
            is_valid_persisted_surveillance_information(operation, information)
        }
        OperationKind::DocumentTheft => {
            is_valid_persisted_document_theft_information(operation, information)
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
        | OperationKind::Arson => false,
    }
}
