//! Persistent delegated criminal enterprises and cycle history; `enterprise_execution` owns lifecycle and routine settlement.

pub mod enterprise_execution;
pub mod enterprise_reporting;

use crate::core::attention::AttentionClass;
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
}

pub const ALL_ENTERPRISE_KINDS: [EnterpriseKind; 6] = [
    EnterpriseKind::Protection,
    EnterpriseKind::Gambling,
    EnterpriseKind::AlcoholDistribution,
    EnterpriseKind::Bookmaking,
    EnterpriseKind::LoanSharking,
    EnterpriseKind::Fencing,
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
    Closed,
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
    /// depends on it and the active-investigation state that produced it changes over time.
    investigation_heat: Money,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EnterpriseCycleArtifacts {
    attention: AttentionClass,
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

    pub fn transaction(&self) -> Option<LedgerTransactionId> {
        self.provenance.transaction
    }

    pub fn information(&self) -> Option<InformationId> {
        self.provenance.information
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnterpriseState {
    records: BTreeMap<EnterpriseId, EnterpriseRecord>,
    cycles: BTreeMap<EnterpriseCycleId, EnterpriseCycleRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<EnterpriseId>>,
    by_manager: BTreeMap<crate::core::id::CharacterId, BTreeSet<EnterpriseId>>,
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

    pub fn enterprises_for_manager(
        &self,
        manager: crate::core::id::CharacterId,
    ) -> impl Iterator<Item = &EnterpriseRecord> {
        self.by_manager
            .get(&manager)
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
    ) -> impl Iterator<Item = &EnterpriseCycleRecord> {
        self.cycles_by_enterprise
            .get(&enterprise)
            .into_iter()
            .flatten()
            .filter_map(|id| self.cycles.get(id))
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

    pub(crate) fn due_at_or_before(&self, now: SimTime) -> Vec<EnterpriseId> {
        self.active_by_next_cycle
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn enterprises(&self) -> impl Iterator<Item = &EnterpriseRecord> {
        self.records.values()
    }

    pub(crate) fn cycles(&self) -> impl Iterator<Item = &EnterpriseCycleRecord> {
        self.cycles.values()
    }

    pub(crate) fn insert(&mut self, record: EnterpriseRecord) {
        let id = record.id();
        self.by_organization
            .entry(record.organization())
            .or_default()
            .insert(id);
        self.by_manager
            .entry(record.manager())
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
                    .by_manager
                    .get(&record.manager())
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
        for (manager, ids) in &self.by_manager {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.manager() == *manager)
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

    #[cfg(debug_assertions)]
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: enterprise indexes disagree with source records"
        );
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
            version: 1,
        },
    }
}
