//! Independent business operating state and durable economic cycle history; `business_economy_system` owns lifecycle and settlement, `business_reporting` is read-only aggregation.

pub mod business_acquisition;
pub mod business_economy_system;
pub mod business_reporting;

use crate::core::attention::AttentionClass;
use crate::core::id::{
    BusinessCycleId, BusinessId, FinancialAccountId, InformationId, LedgerTransactionId,
};
use crate::core::time::SimTime;
use crate::finance::Money;
use crate::world::BusinessOwner;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Business economies run or pause; closure is not a modeled lifecycle (the enterprise domain
/// owns full termination semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusinessOperatingStatus {
    Active,
    Suspended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusinessEconomyRecord {
    business: BusinessId,
    operating_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    status: BusinessOperatingStatus,
    established_at: SimTime,
    next_cycle_at: Option<SimTime>,
    last_cycle_at: Option<SimTime>,
    /// Sabotage damage horizon: while set and not yet passed, cycles earn degraded gross.
    disrupted_through: Option<SimTime>,
    /// Trailing-loss counting starts at this instant. Set when the economy resumes after any
    /// suspension so the authored losing-cycle threshold applies to losses suffered since the
    /// restart instead of resurrecting pre-suspension history on the first losing cycle.
    loss_streak_anchor: Option<SimTime>,
    /// Street-cash volume this front has absorbed since its last operating cycle. The
    /// laundering plausibility budget is per cycle, so splitting one large sum into many
    /// transfers cannot hide more than the front's authored share of legitimate earnings.
    laundered_this_cycle: Money,
    version: u32,
}

impl BusinessEconomyRecord {
    pub fn business(&self) -> BusinessId {
        self.business
    }
    pub fn operating_account(&self) -> FinancialAccountId {
        self.operating_account
    }
    pub fn settlement_account(&self) -> FinancialAccountId {
        self.settlement_account
    }
    pub fn status(&self) -> BusinessOperatingStatus {
        self.status
    }
    pub fn established_at(&self) -> SimTime {
        self.established_at
    }
    pub fn next_cycle_at(&self) -> Option<SimTime> {
        self.next_cycle_at
    }
    pub fn last_cycle_at(&self) -> Option<SimTime> {
        self.last_cycle_at
    }
    #[cfg(test)]
    pub fn disrupted_through(&self) -> Option<SimTime> {
        self.disrupted_through
    }
    pub(crate) fn loss_streak_anchor(&self) -> Option<SimTime> {
        self.loss_streak_anchor
    }
    pub fn is_disrupted(&self, now: SimTime) -> bool {
        self.disrupted_through.is_some_and(|through| now <= through)
    }
    pub fn laundered_this_cycle(&self) -> Money {
        self.laundered_this_cycle
    }
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BusinessCycleContext {
    business: BusinessId,
    business_version: u32,
    owner: BusinessOwner,
    occurred_at: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BusinessCycleFinancials {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BusinessCycleArtifacts {
    attention: AttentionClass,
    transaction: Option<LedgerTransactionId>,
    information: Option<InformationId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusinessCycleRecord {
    id: BusinessCycleId,
    context: BusinessCycleContext,
    financials: BusinessCycleFinancials,
    artifacts: BusinessCycleArtifacts,
}

impl BusinessCycleRecord {
    pub fn id(&self) -> BusinessCycleId {
        self.id
    }
    pub fn business(&self) -> BusinessId {
        self.context.business
    }
    pub fn business_version(&self) -> u32 {
        self.context.business_version
    }
    pub fn owner(&self) -> BusinessOwner {
        self.context.owner
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
    pub fn attention(&self) -> AttentionClass {
        self.artifacts.attention
    }
    pub fn transaction(&self) -> Option<LedgerTransactionId> {
        self.artifacts.transaction
    }
    pub fn information(&self) -> Option<InformationId> {
        self.artifacts.information
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EconomyState {
    businesses: BTreeMap<BusinessId, BusinessEconomyRecord>,
    cycles: BTreeMap<BusinessCycleId, BusinessCycleRecord>,
    active_by_next_cycle: BTreeMap<SimTime, BTreeSet<BusinessId>>,
    by_settlement_account: BTreeMap<FinancialAccountId, BusinessId>,
    cycles_by_business: BTreeMap<BusinessId, BTreeSet<BusinessCycleId>>,
}

impl EconomyState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_business_economy(&self, business: BusinessId) -> Option<&BusinessEconomyRecord> {
        self.businesses.get(&business)
    }

    pub fn get_cycle(&self, id: BusinessCycleId) -> Option<&BusinessCycleRecord> {
        self.cycles.get(&id)
    }

    pub fn cycles_for(
        &self,
        business: BusinessId,
    ) -> impl DoubleEndedIterator<Item = &BusinessCycleRecord> {
        // Cycle IDs are allocated sequentially, so index order is settlement order; the
        // double-ended bound lets consumers scan only the newest history.
        self.cycles_by_business
            .get(&business)
            .into_iter()
            .flatten()
            .map(|id| self.cycles.get(id).expect("indexed cycle must exist"))
    }

    pub fn get_by_settlement_account(
        &self,
        account: FinancialAccountId,
    ) -> Option<&BusinessEconomyRecord> {
        self.by_settlement_account
            .get(&account)
            .and_then(|business| self.businesses.get(business))
    }

    pub(crate) fn due_at_or_before(&self, now: SimTime) -> Vec<BusinessId> {
        self.active_by_next_cycle
            .range(..=now)
            .flat_map(|(_, businesses)| businesses.iter().copied())
            .collect()
    }

    pub(crate) fn business_economies(&self) -> impl Iterator<Item = &BusinessEconomyRecord> {
        self.businesses.values()
    }

    pub(crate) fn cycles(&self) -> impl Iterator<Item = &BusinessCycleRecord> {
        self.cycles.values()
    }

    pub(crate) fn insert(&mut self, record: BusinessEconomyRecord) {
        let business = record.business();
        let next_cycle_at = record
            .next_cycle_at()
            .expect("new active business economy must have a scheduled cycle");
        self.active_by_next_cycle
            .entry(next_cycle_at)
            .or_default()
            .insert(business);
        let previous_settlement = self
            .by_settlement_account
            .insert(record.settlement_account(), business);
        debug_assert!(
            previous_settlement.is_none(),
            "Ownership Exclusivity: business settlement account is already assigned"
        );
        let previous = self.businesses.insert(business, record);
        debug_assert!(
            previous.is_none(),
            "Ownership Exclusivity: duplicate business economy inserted"
        );
    }

    pub(crate) fn apply_cycle(&mut self, cycle: BusinessCycleRecord, next_cycle_at: SimTime) {
        let business = cycle.business();
        let old_next_cycle_at = self
            .businesses
            .get(&business)
            .expect("validated business economy disappeared before cycle commit")
            .next_cycle_at()
            .expect("active business economy must have a scheduled cycle");
        Self::remove_schedule_index(&mut self.active_by_next_cycle, old_next_cycle_at, business);
        let record = self
            .businesses
            .get_mut(&business)
            .expect("validated business economy disappeared before cycle commit");
        record.last_cycle_at = Some(cycle.occurred_at());
        record.next_cycle_at = Some(next_cycle_at);
        // A new operating cycle starts a fresh laundering plausibility window.
        record.laundered_this_cycle = Money::ZERO;
        record.version = record
            .version
            .checked_add(1)
            .expect("business economy version counter exhausted");
        self.active_by_next_cycle
            .entry(next_cycle_at)
            .or_default()
            .insert(business);
        self.cycles_by_business
            .entry(business)
            .or_default()
            .insert(cycle.id());
        let previous = self.cycles.insert(cycle.id(), cycle);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate business cycle ID inserted"
        );
    }

    pub(crate) fn set_status(
        &mut self,
        business: BusinessId,
        status: BusinessOperatingStatus,
        next_cycle_at: Option<SimTime>,
        // Installed only when the economy returns to Active: restarts the chronic-loss grace
        // window so pre-suspension losses cannot instantly re-suspend a resumed business.
        loss_streak_anchor: Option<SimTime>,
    ) {
        let (was_active, old_next_cycle_at) = {
            let record = self
                .businesses
                .get(&business)
                .expect("validated business economy disappeared before status commit");
            (
                record.status() == BusinessOperatingStatus::Active,
                record.next_cycle_at(),
            )
        };
        let will_be_active = status == BusinessOperatingStatus::Active;
        if was_active {
            Self::remove_schedule_index(
                &mut self.active_by_next_cycle,
                old_next_cycle_at.expect("active business economy must be scheduled"),
                business,
            );
        }
        if will_be_active {
            self.active_by_next_cycle
                .entry(next_cycle_at.expect("active business economy must be rescheduled"))
                .or_default()
                .insert(business);
        }
        let record = self
            .businesses
            .get_mut(&business)
            .expect("validated business economy disappeared before status commit");
        record.status = status;
        record.next_cycle_at = next_cycle_at;
        if let Some(anchor) = loss_streak_anchor {
            record.loss_streak_anchor = Some(anchor);
        }
        record.version = record
            .version
            .checked_add(1)
            .expect("business economy version counter exhausted");
    }

    /// Extends the sabotage damage horizon for a business economy. The horizon is monotone:
    /// a new disruption only ever pushes the horizon later, never restores it early.
    pub(crate) fn apply_disruption(&mut self, business: BusinessId, disrupted_through: SimTime) {
        let record = self
            .businesses
            .get_mut(&business)
            .expect("validated business economy disappeared before disruption commit");
        record.disrupted_through = Some(match record.disrupted_through {
            Some(current) if current > disrupted_through => current,
            _ => disrupted_through,
        });
        record.version = record
            .version
            .checked_add(1)
            .expect("business economy version counter exhausted");
    }

    /// Records laundered street-cash volume absorbed by this front in its current cycle. The
    /// economy owner keeps the running total so laundering plausibility stays a per-cycle
    /// budget rather than a per-transfer allowance.
    pub(crate) fn record_laundered_volume(&mut self, business: BusinessId, amount: Money) {
        let record = self
            .businesses
            .get_mut(&business)
            .expect("validated front business economy disappeared before laundering commit");
        record.laundered_this_cycle = record
            .laundered_this_cycle
            .checked_add(amount)
            .expect("laundered-volume accumulator overflowed");
        record.version = record
            .version
            .checked_add(1)
            .expect("business economy version counter exhausted");
    }

    fn remove_schedule_index(
        index: &mut BTreeMap<SimTime, BTreeSet<BusinessId>>,
        time: SimTime,
        business: BusinessId,
    ) {
        if let Some(businesses) = index.get_mut(&time) {
            businesses.remove(&business);
            if businesses.is_empty() {
                index.remove(&time);
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.businesses.values() {
            if self.by_settlement_account.get(&record.settlement_account())
                != Some(&record.business())
            {
                return false;
            }
            let scheduled = record.next_cycle_at().is_some_and(|time| {
                self.active_by_next_cycle
                    .get(&time)
                    .is_some_and(|businesses| businesses.contains(&record.business()))
            });
            if scheduled != (record.status() == BusinessOperatingStatus::Active) {
                return false;
            }
        }
        for (account, business) in &self.by_settlement_account {
            if !self
                .businesses
                .get(business)
                .is_some_and(|record| record.settlement_account() == *account)
            {
                return false;
            }
        }
        for (time, businesses) in &self.active_by_next_cycle {
            for business in businesses {
                if !self.businesses.get(business).is_some_and(|record| {
                    record.status() == BusinessOperatingStatus::Active
                        && record.next_cycle_at() == Some(*time)
                }) {
                    return false;
                }
            }
        }
        for cycle in self.cycles.values() {
            if !self
                .cycles_by_business
                .get(&cycle.business())
                .is_some_and(|ids| ids.contains(&cycle.id()))
            {
                return false;
            }
        }
        for (business, ids) in &self.cycles_by_business {
            for id in ids {
                if !self
                    .cycles
                    .get(id)
                    .is_some_and(|cycle| cycle.business() == *business)
                {
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
            "Derived Data Consistency: business economy indexes disagree with source records"
        );
    }
}

pub struct BusinessEconomyDraft {
    pub business: BusinessId,
    pub operating_account: FinancialAccountId,
    pub settlement_account: FinancialAccountId,
}

pub(crate) fn build_business_economy_record(
    draft: BusinessEconomyDraft,
    established_at: SimTime,
    next_cycle_at: SimTime,
) -> BusinessEconomyRecord {
    BusinessEconomyRecord {
        business: draft.business,
        operating_account: draft.operating_account,
        settlement_account: draft.settlement_account,
        status: BusinessOperatingStatus::Active,
        established_at,
        next_cycle_at: Some(next_cycle_at),
        last_cycle_at: None,
        disrupted_through: None,
        loss_streak_anchor: None,
        laundered_this_cycle: Money::ZERO,
        version: 1,
    }
}
