//! Persistent delegated criminal-enterprise records and cycle history; `enterprise_state` owns
//! storage/index maintenance, `enterprise_execution` is the lifecycle/settlement facade, its
//! `cycle_planning` sibling owns read-only cycle decisions, `autonomous_planning` owns shared
//! NPC projections, `autonomous_lifecycle` owns suspended-racket maintenance,
//! `autonomous_expansion` owns new delegated growth, and `enterprise_reporting` is read-only
//! aggregation.

pub(crate) mod autonomous_expansion;
pub(crate) mod autonomous_lifecycle;
pub(crate) mod autonomous_planning;
pub mod enterprise_execution;
pub mod enterprise_reporting;
mod enterprise_state;

pub use enterprise_state::EnterpriseState;

use crate::core::attention::AttentionClass;
use crate::core::id::{
    BusinessId, EnterpriseCycleId, EnterpriseId, FinancialAccountId, InformationId,
    LedgerTransactionId, NeighborhoodId, OrganizationId,
};
use crate::core::time::SimTime;
use crate::delegation::MandateAuthority;
use crate::finance::Money;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EnterpriseKind {
    Protection,
    Gambling,
    AlcoholDistribution,
    Bookmaking,
    LoanSharking,
    Fencing,
    Speakeasy,
    LaborRacketeering,
    NumbersRacket,
    SlotMachineRoute,
    Brothel,
    PrizeFighting,
    Counterfeiting,
    Fraud,
    AutoTheftRing,
    Smuggling,
}

pub const ALL_ENTERPRISE_KINDS: [EnterpriseKind; 16] = [
    EnterpriseKind::Protection,
    EnterpriseKind::Gambling,
    EnterpriseKind::AlcoholDistribution,
    EnterpriseKind::Bookmaking,
    EnterpriseKind::LoanSharking,
    EnterpriseKind::Fencing,
    EnterpriseKind::Speakeasy,
    EnterpriseKind::LaborRacketeering,
    EnterpriseKind::NumbersRacket,
    EnterpriseKind::SlotMachineRoute,
    EnterpriseKind::Brothel,
    EnterpriseKind::PrizeFighting,
    EnterpriseKind::Counterfeiting,
    EnterpriseKind::Fraud,
    EnterpriseKind::AutoTheftRing,
    EnterpriseKind::Smuggling,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EnterpriseLocation {
    Neighborhood(NeighborhoodId),
    Business(crate::core::id::BusinessId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnterpriseStatus {
    Active,
    Suspended,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EnterpriseIdentity {
    id: EnterpriseId,
    kind: EnterpriseKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EnterpriseAssignment {
    organization: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
    supporting_businesses: BTreeSet<BusinessId>,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EnterpriseRuntime {
    status: EnterpriseStatus,
    established_at: SimTime,
    /// Terminal lifecycle boundary. Retired enterprises remain durable history, but reporting
    /// and other historical projections need the exact instant at which they stopped existing
    /// as live rackets rather than inferring chronology from their current terminal status.
    retired_at: Option<SimTime>,
    next_cycle_at: Option<SimTime>,
    last_cycle_at: Option<SimTime>,
    /// Trailing-loss counting starts at this instant. Set when the racket resumes after any
    /// suspension so the authored losing-cycle threshold applies to losses suffered since the
    /// restart instead of resurrecting pre-suspension history on the first losing cycle.
    loss_streak_anchor: Option<SimTime>,
    version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnterpriseRecord {
    identity: EnterpriseIdentity,
    assignment: EnterpriseAssignment,
    runtime: EnterpriseRuntime,
}

impl EnterpriseRecord {
    pub fn id(&self) -> EnterpriseId {
        self.identity.id
    }

    pub fn kind(&self) -> EnterpriseKind {
        self.identity.kind
    }

    pub fn organization(&self) -> OrganizationId {
        self.assignment.organization
    }

    pub fn authority(&self) -> MandateAuthority {
        self.assignment.authority
    }

    pub fn manager(&self) -> crate::core::id::CharacterId {
        self.assignment.authority.manager
    }

    pub fn location(&self) -> EnterpriseLocation {
        self.assignment.location
    }

    pub fn supporting_businesses(&self) -> &BTreeSet<BusinessId> {
        &self.assignment.supporting_businesses
    }

    pub fn cash_account(&self) -> FinancialAccountId {
        self.assignment.cash_account
    }

    pub fn settlement_account(&self) -> FinancialAccountId {
        self.assignment.settlement_account
    }

    pub fn status(&self) -> EnterpriseStatus {
        self.runtime.status
    }

    pub fn established_at(&self) -> SimTime {
        self.runtime.established_at
    }

    pub fn retired_at(&self) -> Option<SimTime> {
        self.runtime.retired_at
    }

    pub fn next_cycle_at(&self) -> Option<SimTime> {
        self.runtime.next_cycle_at
    }

    pub fn last_cycle_at(&self) -> Option<SimTime> {
        self.runtime.last_cycle_at
    }

    pub(crate) fn loss_streak_anchor(&self) -> Option<SimTime> {
        self.runtime.loss_streak_anchor
    }

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleContext {
    enterprise: EnterpriseId,
    occurred_at: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleFinancials {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    /// Street-heat portion of `operating_cost` at settlement. Persisted because notability
    /// depends on it and on the previous cycle's heat, while the active-investigation state
    /// that produced both changes over time.
    investigation_heat: Money,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleArtifacts {
    attention: AttentionClass,
    /// Set when this settlement drew a racket inquiry onto the racket: sustained district
    /// casework opened a police investigation or reactivated a matching suspended shelf under
    /// the current intake authority.
    drew_enforcement_attention: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleProvenance {
    transaction: Option<LedgerTransactionId>,
    information: Option<InformationId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnterpriseCycleRecord {
    id: EnterpriseCycleId,
    context: EnterpriseCycleContext,
    financials: EnterpriseCycleFinancials,
    artifacts: EnterpriseCycleArtifacts,
    provenance: EnterpriseCycleProvenance,
}

impl EnterpriseCycleRecord {
    pub fn id(&self) -> EnterpriseCycleId {
        self.id
    }

    pub fn enterprise(&self) -> EnterpriseId {
        self.context.enterprise
    }

    pub fn occurred_at(&self) -> SimTime {
        self.context.occurred_at
    }

    pub fn gross_revenue(&self) -> Money {
        self.financials.gross_revenue
    }

    pub fn operating_cost(&self) -> Money {
        self.financials.operating_cost
    }

    pub fn net_cash(&self) -> Money {
        self.financials.net_cash
    }

    pub fn variance_basis_points(&self) -> i16 {
        self.financials.variance_basis_points
    }

    pub fn investigation_heat(&self) -> Money {
        self.financials.investigation_heat
    }

    pub fn attention(&self) -> AttentionClass {
        self.artifacts.attention
    }

    pub fn drew_enforcement_attention(&self) -> bool {
        self.artifacts.drew_enforcement_attention
    }

    pub fn transaction(&self) -> Option<LedgerTransactionId> {
        self.provenance.transaction
    }

    pub fn information(&self) -> Option<InformationId> {
        self.provenance.information
    }
}

#[derive(Clone, Debug)]
pub struct EnterpriseDraft {
    pub kind: EnterpriseKind,
    pub organization: OrganizationId,
    pub authority: MandateAuthority,
    pub location: EnterpriseLocation,
    pub supporting_businesses: BTreeSet<BusinessId>,
    pub cash_account: FinancialAccountId,
    pub settlement_account: FinancialAccountId,
}

fn build_enterprise_record(
    id: EnterpriseId,
    draft: EnterpriseDraft,
    established_at: SimTime,
    next_cycle_at: SimTime,
) -> EnterpriseRecord {
    let EnterpriseDraft {
        kind,
        organization,
        authority,
        location,
        supporting_businesses,
        cash_account,
        settlement_account,
    } = draft;
    EnterpriseRecord {
        identity: EnterpriseIdentity { id, kind },
        assignment: EnterpriseAssignment {
            organization,
            authority,
            location,
            supporting_businesses,
            cash_account,
            settlement_account,
        },
        runtime: EnterpriseRuntime {
            status: EnterpriseStatus::Active,
            established_at,
            retired_at: None,
            next_cycle_at: Some(next_cycle_at),
            last_cycle_at: None,
            loss_streak_anchor: None,
            version: 1,
        },
    }
}
