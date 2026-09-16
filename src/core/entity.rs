//! Cross-subsystem entity references used by intelligence, history, reports, and investigations.

use crate::core::id::{
    BusinessId, CharacterId, DecisionRequestId, EnterpriseId, EvidenceId, FinancialAccountId,
    InvestigationId, MandateId, NeighborhoodId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntityRef {
    Organization(OrganizationId),
    Character(CharacterId),
    Neighborhood(NeighborhoodId),
    Business(BusinessId),
    Operation(OperationId),
    Investigation(InvestigationId),
    Evidence(EvidenceId),
    FinancialAccount(FinancialAccountId),
    DecisionRequest(DecisionRequestId),
    Mandate(MandateId),
    Enterprise(EnterpriseId),
}

impl EntityRef {
    /// The character this reference names, if it names one. Centralizes the character-vs-other
    /// split so new variants fail review at one site instead of drifting across duplicated
    /// eleven-arm matches in every consumer.
    pub(crate) const fn as_character(self) -> Option<CharacterId> {
        match self {
            Self::Character(character) => Some(character),
            Self::Organization(_)
            | Self::Neighborhood(_)
            | Self::Business(_)
            | Self::Operation(_)
            | Self::Investigation(_)
            | Self::Evidence(_)
            | Self::FinancialAccount(_)
            | Self::DecisionRequest(_)
            | Self::Mandate(_)
            | Self::Enterprise(_) => None,
        }
    }
}

pub(crate) fn is_entity_present(state: &AppState, entity: EntityRef) -> bool {
    match entity {
        EntityRef::Organization(id) => state.world.get_organization(id).is_some(),
        EntityRef::Character(id) => state.world.get_character(id).is_some(),
        EntityRef::Neighborhood(id) => state.world.get_neighborhood(id).is_some(),
        EntityRef::Business(id) => state.world.get_business(id).is_some(),
        EntityRef::Operation(id) => state.operations.get_operation(id).is_some(),
        EntityRef::Investigation(id) => state.legal.get_investigation(id).is_some(),
        EntityRef::Evidence(id) => state.legal.get_evidence(id).is_some(),
        EntityRef::FinancialAccount(id) => state.finance.get_account(id).is_some(),
        EntityRef::DecisionRequest(id) => state.decisions.get_decision(id).is_some(),
        EntityRef::Mandate(id) => state.delegation.get_mandate(id).is_some(),
        EntityRef::Enterprise(id) => state.enterprises.get_enterprise(id).is_some(),
    }
}
