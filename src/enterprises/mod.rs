//! Persistent delegated criminal enterprises and cycle history; `enterprise_execution` owns lifecycle and routine settlement.

pub mod autonomous_expansion;
pub mod enterprise_execution;
pub mod enterprise_reporting;

use crate::core::attention::AttentionClass;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    BusinessId, EnterpriseCycleId, EnterpriseId, FinancialAccountId, InformationId,
    LedgerTransactionId, MandateId, NeighborhoodId, OrganizationId,
};
use crate::core::time::SimTime;
use crate::delegation::MandateAuthority;
use crate::finance::Money;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
}

pub const ALL_ENTERPRISE_KINDS: [EnterpriseKind; 8] = [
    EnterpriseKind::Protection,
    EnterpriseKind::Gambling,
    EnterpriseKind::AlcoholDistribution,
    EnterpriseKind::Bookmaking,
    EnterpriseKind::LoanSharking,
    EnterpriseKind::Fencing,
    EnterpriseKind::Speakeasy,
    EnterpriseKind::LaborRacketeering,
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
    /// Set when this settlement drew a vice inquiry onto the racket: sustained district
    /// casework converted into a new police investigation owned by the intake authority.
    drew_vice_attention: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleProvenance {
    transaction: Option<LedgerTransactionId>,
    information: Option<InformationId>,
    /// Organization-facing legal-activity knowledge created when this cycle drew a vice
    /// inquiry; `None` whenever no inquiry was opened.
    vice_information: Option<InformationId>,
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

    pub fn drew_vice_attention(&self) -> bool {
        self.artifacts.drew_vice_attention
    }

    pub fn transaction(&self) -> Option<LedgerTransactionId> {
        self.provenance.transaction
    }

    pub fn information(&self) -> Option<InformationId> {
        self.provenance.information
    }

    pub fn vice_information(&self) -> Option<InformationId> {
        self.provenance.vice_information
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnterpriseState {
    records: BTreeMap<EnterpriseId, EnterpriseRecord>,
    cycles: BTreeMap<EnterpriseCycleId, EnterpriseCycleRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<EnterpriseId>>,
    by_location: BTreeMap<EnterpriseLocation, BTreeSet<EnterpriseId>>,
    by_supporting_business: BTreeMap<BusinessId, BTreeSet<EnterpriseId>>,
    active_by_mandate: BTreeMap<MandateId, BTreeSet<EnterpriseId>>,
    active_by_next_cycle: BTreeMap<SimTime, BTreeSet<EnterpriseId>>,
    by_settlement_account: BTreeMap<FinancialAccountId, EnterpriseId>,
    cycles_by_enterprise: BTreeMap<EnterpriseId, BTreeSet<EnterpriseCycleId>>,
}

impl EnterpriseState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_enterprise(&self, id: EnterpriseId) -> Option<&EnterpriseRecord> {
        self.records.get(&id)
    }

    pub fn get_cycle(&self, id: EnterpriseCycleId) -> Option<&EnterpriseCycleRecord> {
        self.cycles.get(&id)
    }

    pub fn enterprises_for_organization(
        &self,
        organization: OrganizationId,
    ) -> impl Iterator<Item = &EnterpriseRecord> {
        self.by_organization
            .get(&organization)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn enterprises_at(
        &self,
        location: EnterpriseLocation,
    ) -> impl Iterator<Item = &EnterpriseRecord> {
        self.by_location
            .get(&location)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn enterprises_supported_by_business(
        &self,
        business: BusinessId,
    ) -> impl Iterator<Item = &EnterpriseRecord> {
        self.by_supporting_business
            .get(&business)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn cycles_for(
        &self,
        enterprise: EnterpriseId,
    ) -> impl DoubleEndedIterator<Item = &EnterpriseCycleRecord> {
        // Cycle IDs are allocated sequentially, so index order is settlement order; the
        // double-ended bound lets consumers scan only the newest history.
        self.cycles_by_enterprise
            .get(&enterprise)
            .into_iter()
            .flatten()
            .map(|id| self.cycles.get(id).expect("indexed cycle must exist"))
    }

    /// The most recent settled cycle for an enterprise, in O(log n): settlement order is
    /// sequential-ID order, so the last indexed ID is the newest cycle.
    pub fn latest_cycle(&self, enterprise: EnterpriseId) -> Option<&EnterpriseCycleRecord> {
        self.cycles_by_enterprise
            .get(&enterprise)?
            .last()
            .and_then(|id| self.cycles.get(id))
    }

    /// The settled cycle immediately preceding `cycle` for its enterprise, in O(log n).
    /// Sequential-ID settlement order makes the highest ID below `cycle` the prior cycle;
    /// this is what per-cycle reportability comparisons need without walking the history.
    pub fn prior_cycle(
        &self,
        enterprise: EnterpriseId,
        cycle: EnterpriseCycleId,
    ) -> Option<&EnterpriseCycleRecord> {
        self.cycles_by_enterprise
            .get(&enterprise)?
            .range(..cycle)
            .next_back()
            .and_then(|id| self.cycles.get(id))
    }

    pub fn active_for_mandate(
        &self,
        mandate: MandateId,
    ) -> impl Iterator<Item = &EnterpriseRecord> {
        self.active_by_mandate
            .get(&mandate)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn get_by_settlement_account(
        &self,
        account: FinancialAccountId,
    ) -> Option<&EnterpriseRecord> {
        self.by_settlement_account
            .get(&account)
            .and_then(|id| self.records.get(id))
    }

    pub(crate) fn find_due_cycles(&self, now: SimTime) -> Vec<EnterpriseId> {
        let mut due: Vec<EnterpriseId> = self
            .active_by_next_cycle
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        due.sort_unstable();
        due
    }

    pub(crate) fn enterprises(&self) -> impl Iterator<Item = &EnterpriseRecord> {
        self.records.values()
    }
    pub(crate) fn enterprise_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }

    pub(crate) fn cycles(&self) -> impl Iterator<Item = &EnterpriseCycleRecord> {
        self.cycles.values()
    }
    pub(crate) fn enterprise_cycle_id_bounds(&self) -> Option<(u32, u32)> {
        self.cycles.id_bounds()
    }

    pub(crate) fn insert(&mut self, record: EnterpriseRecord) {
        let id = record.id();
        self.by_organization
            .entry(record.organization())
            .or_default()
            .insert(id);
        self.by_location
            .entry(record.location())
            .or_default()
            .insert(id);
        for business in record.supporting_businesses() {
            self.by_supporting_business
                .entry(*business)
                .or_default()
                .insert(id);
        }
        self.active_by_mandate
            .entry(record.authority().mandate)
            .or_default()
            .insert(id);
        let next_cycle_at = record
            .next_cycle_at()
            .expect("new active enterprise must have a scheduled cycle");
        self.active_by_next_cycle
            .entry(next_cycle_at)
            .or_default()
            .insert(id);
        let previous_settlement = self
            .by_settlement_account
            .insert(record.settlement_account(), id);
        debug_assert!(
            previous_settlement.is_none(),
            "Ownership Exclusivity: enterprise settlement account is already assigned"
        );
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate enterprise ID inserted"
        );
    }

    pub(crate) fn apply_cycle(&mut self, cycle: EnterpriseCycleRecord, next_cycle_at: SimTime) {
        let enterprise_id = cycle.enterprise();
        let old_next_cycle_at = self
            .records
            .get(&enterprise_id)
            .expect("validated enterprise disappeared before cycle commit")
            .next_cycle_at()
            .expect("active enterprise must have a scheduled cycle");
        Self::remove_schedule_index(
            &mut self.active_by_next_cycle,
            old_next_cycle_at,
            enterprise_id,
        );
        let enterprise = self
            .records
            .get_mut(&enterprise_id)
            .expect("validated enterprise disappeared before cycle commit");
        enterprise.runtime.last_cycle_at = Some(cycle.occurred_at());
        enterprise.runtime.next_cycle_at = Some(next_cycle_at);
        enterprise.runtime.version = enterprise
            .runtime
            .version
            .checked_add(1)
            .expect("enterprise version counter exhausted");
        self.active_by_next_cycle
            .entry(next_cycle_at)
            .or_default()
            .insert(enterprise_id);
        self.cycles_by_enterprise
            .entry(enterprise_id)
            .or_default()
            .insert(cycle.id());
        let previous = self.cycles.insert(cycle.id(), cycle);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate enterprise cycle ID inserted"
        );
    }

    pub(crate) fn set_status(
        &mut self,
        id: EnterpriseId,
        status: EnterpriseStatus,
        next_cycle_at: Option<SimTime>,
        // Installed only when the enterprise returns to Active: restarts the chronic-loss
        // grace window so pre-suspension losses cannot instantly re-suspend a resumed racket.
        loss_streak_anchor: Option<SimTime>,
    ) {
        let (was_active, mandate, old_next_cycle_at) = {
            let record = self
                .records
                .get(&id)
                .expect("validated enterprise disappeared before status commit");
            (
                record.runtime.status == EnterpriseStatus::Active,
                record.assignment.authority.mandate,
                record.runtime.next_cycle_at,
            )
        };
        let will_be_active = status == EnterpriseStatus::Active;
        if was_active {
            let old_next_cycle_at = old_next_cycle_at
                .expect("active enterprise must have a scheduled cycle before status change");
            Self::remove_schedule_index(&mut self.active_by_next_cycle, old_next_cycle_at, id);
        }
        if was_active && !will_be_active {
            Self::remove_active_mandate_index(&mut self.active_by_mandate, mandate, id);
        } else if !was_active && will_be_active {
            self.active_by_mandate
                .entry(mandate)
                .or_default()
                .insert(id);
        }
        if will_be_active {
            let scheduled = next_cycle_at
                .expect("active enterprise status change must include next cycle time");
            self.active_by_next_cycle
                .entry(scheduled)
                .or_default()
                .insert(id);
        }
        let record = self
            .records
            .get_mut(&id)
            .expect("validated enterprise disappeared before status commit");
        record.runtime.status = status;
        record.runtime.next_cycle_at = next_cycle_at;
        if let Some(anchor) = loss_streak_anchor {
            record.runtime.loss_streak_anchor = Some(anchor);
        }
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("enterprise version counter exhausted");
    }

    fn remove_active_mandate_index(
        index: &mut BTreeMap<MandateId, BTreeSet<EnterpriseId>>,
        mandate: MandateId,
        enterprise: EnterpriseId,
    ) {
        if let Some(ids) = index.get_mut(&mandate) {
            ids.remove(&enterprise);
            if ids.is_empty() {
                index.remove(&mandate);
            }
        }
    }

    fn remove_schedule_index(
        index: &mut BTreeMap<SimTime, BTreeSet<EnterpriseId>>,
        time: SimTime,
        enterprise: EnterpriseId,
    ) {
        if let Some(ids) = index.get_mut(&time) {
            ids.remove(&enterprise);
            if ids.is_empty() {
                index.remove(&time);
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            if !self
                .by_organization
                .get(&record.organization())
                .is_some_and(|ids| ids.contains(&record.id()))
                || !self
                    .by_location
                    .get(&record.location())
                    .is_some_and(|ids| ids.contains(&record.id()))
                || record.supporting_businesses().iter().any(|business| {
                    !self
                        .by_supporting_business
                        .get(business)
                        .is_some_and(|ids| ids.contains(&record.id()))
                })
                || self.by_settlement_account.get(&record.settlement_account())
                    != Some(&record.id())
            {
                return false;
            }
            let is_active_indexed = self
                .active_by_mandate
                .get(&record.authority().mandate)
                .is_some_and(|ids| ids.contains(&record.id()));
            if is_active_indexed != (record.status() == EnterpriseStatus::Active) {
                return false;
            }
            let is_schedule_indexed = record.next_cycle_at().is_some_and(|time| {
                self.active_by_next_cycle
                    .get(&time)
                    .is_some_and(|ids| ids.contains(&record.id()))
            });
            if is_schedule_indexed != (record.status() == EnterpriseStatus::Active) {
                return false;
            }
        }
        for (organization, ids) in &self.by_organization {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.organization() == *organization)
                {
                    return false;
                }
            }
        }
        for (location, ids) in &self.by_location {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.location() == *location)
                {
                    return false;
                }
            }
        }
        for (business, ids) in &self.by_supporting_business {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.supporting_businesses().contains(business))
                {
                    return false;
                }
            }
        }
        for (account, id) in &self.by_settlement_account {
            if !self
                .records
                .get(id)
                .is_some_and(|record| record.settlement_account() == *account)
            {
                return false;
            }
        }
        for cycle in self.cycles.values() {
            if !self
                .cycles_by_enterprise
                .get(&cycle.enterprise())
                .is_some_and(|ids| ids.contains(&cycle.id()))
            {
                return false;
            }
        }
        for (enterprise, ids) in &self.cycles_by_enterprise {
            for id in ids {
                if !self
                    .cycles
                    .get(id)
                    .is_some_and(|cycle| cycle.enterprise() == *enterprise)
                {
                    return false;
                }
            }
        }
        for (mandate, ids) in &self.active_by_mandate {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == EnterpriseStatus::Active
                        && record.authority().mandate == *mandate
                }) {
                    return false;
                }
            }
        }
        for (time, ids) in &self.active_by_next_cycle {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == EnterpriseStatus::Active
                        && record.next_cycle_at() == Some(*time)
                }) {
                    return false;
                }
            }
        }
        true
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

pub(crate) fn build_enterprise_record(
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
            next_cycle_at: Some(next_cycle_at),
            last_cycle_at: None,
            loss_streak_anchor: None,
            version: 1,
        },
    }
}
