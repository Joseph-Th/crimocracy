//! Persistent manager mandates and responsibility indexes; `delegation_system` owns assignment, revision, and revocation.

pub mod delegation_system;

use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    BusinessId, CharacterId, FinancialAccountId, MandateId, NeighborhoodId, OrganizationId,
};
use crate::core::time::SimTime;
use crate::finance::Money;
use crate::world::{PolicyKind, PolicySetting};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResponsibilityFunction {
    Enterprise,
    Territory,
    Operations,
    Intelligence,
    Finance,
    Legal,
    Political,
    Personnel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResponsibilityScope {
    Neighborhood(NeighborhoodId),
    Business(BusinessId),
    Function(ResponsibilityFunction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateAuthority {
    pub mandate: MandateId,
    pub manager: CharacterId,
    pub scope: ResponsibilityScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedMandateAuthority {
    authority: MandateAuthority,
    organization: OrganizationId,
    mandate_version: u32,
    manager_version: u32,
}

impl ResolvedMandateAuthority {
    pub fn authority(self) -> MandateAuthority {
        self.authority
    }

    pub fn organization(self) -> OrganizationId {
        self.organization
    }

    pub fn mandate_version(self) -> u32 {
        self.mandate_version
    }

    pub fn manager_version(self) -> u32 {
        self.manager_version
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MandateStatus {
    Active,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetPeriod {
    Daily,
    Weekly,
}

impl BudgetPeriod {
    pub const fn duration_minutes(self) -> u64 {
        match self {
            Self::Daily => 1_440,
            Self::Weekly => 10_080,
        }
    }

    pub fn window(self, time: SimTime) -> BudgetWindow {
        let duration = self.duration_minutes();
        let start_minutes = (time.as_minutes() / duration) * duration;
        BudgetWindow {
            start: SimTime::from_minutes(start_minutes),
            end: SimTime::from_minutes(
                start_minutes
                    .checked_add(duration)
                    .expect("budget period end overflowed simulation time"),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetWindow {
    start: SimTime,
    end: SimTime,
}

impl BudgetWindow {
    pub fn start(self) -> SimTime {
        self.start
    }

    pub fn end(self) -> SimTime {
        self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetAuthority {
    pub funding_account: FinancialAccountId,
    pub limit: Money,
    pub period: BudgetPeriod,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MandateRecord {
    id: MandateId,
    organization: OrganizationId,
    manager: CharacterId,
    scopes: BTreeSet<ResponsibilityScope>,
    standing_orders: BTreeMap<PolicyKind, PolicySetting>,
    budget: Option<BudgetAuthority>,
    status: MandateStatus,
    version: u32,
}

impl MandateRecord {
    pub fn id(&self) -> MandateId {
        self.id
    }

    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn manager(&self) -> CharacterId {
        self.manager
    }

    pub fn scopes(&self) -> &BTreeSet<ResponsibilityScope> {
        &self.scopes
    }

    pub fn standing_order(&self, kind: PolicyKind) -> Option<PolicySetting> {
        self.standing_orders.get(&kind).copied()
    }

    pub fn standing_orders(&self) -> &BTreeMap<PolicyKind, PolicySetting> {
        &self.standing_orders
    }

    pub fn budget(&self) -> Option<BudgetAuthority> {
        self.budget
    }

    pub fn status(&self) -> MandateStatus {
        self.status
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DelegationState {
    records: BTreeMap<MandateId, MandateRecord>,
    active_by_manager: BTreeMap<CharacterId, MandateId>,
    active_by_scope: BTreeMap<ResponsibilityScope, BTreeSet<MandateId>>,
    /// Every active mandate by id, so per-day autonomy passes iterate governed mandates
    /// without rescanning the revoked-and-active mandate history.
    active: BTreeSet<MandateId>,
}

impl DelegationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_mandate(&self, id: MandateId) -> Option<&MandateRecord> {
        self.records.get(&id)
    }

    pub fn active_for_manager(&self, manager: CharacterId) -> Option<&MandateRecord> {
        self.active_by_manager
            .get(&manager)
            .and_then(|id| self.records.get(id))
    }

    pub fn active_for_scope(
        &self,
        scope: ResponsibilityScope,
    ) -> impl Iterator<Item = &MandateRecord> {
        self.active_by_scope
            .get(&scope)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub(crate) fn mandates(&self) -> impl Iterator<Item = &MandateRecord> {
        self.records.values()
    }
    /// Every active mandate in id order; daily autonomy passes scan this instead of the
    /// full revoked-and-active mandate history.
    pub(crate) fn active_mandates(&self) -> impl Iterator<Item = &MandateRecord> {
        self.active.iter().filter_map(|id| self.records.get(id))
    }
    pub(crate) fn mandate_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }

    pub(crate) fn insert(&mut self, record: MandateRecord) {
        let id = record.id();
        let previous_manager = self.active_by_manager.insert(record.manager(), id);
        debug_assert!(
            previous_manager.is_none(),
            "Index Uniqueness: manager already holds an active mandate"
        );
        for scope in record.scopes() {
            self.active_by_scope.entry(*scope).or_default().insert(id);
        }
        self.active.insert(id);
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate mandate ID inserted"
        );
    }

    pub(crate) fn revise(
        &mut self,
        id: MandateId,
        scopes: BTreeSet<ResponsibilityScope>,
        standing_orders: BTreeMap<PolicyKind, PolicySetting>,
        budget: Option<BudgetAuthority>,
    ) {
        let old_scopes = self
            .records
            .get(&id)
            .expect("validated mandate disappeared before revision commit")
            .scopes
            .clone();
        for scope in old_scopes {
            Self::remove_scope_index(&mut self.active_by_scope, scope, id);
        }

        let record = self
            .records
            .get_mut(&id)
            .expect("validated mandate disappeared before revision commit");
        record.scopes = scopes;
        record.standing_orders = standing_orders;
        record.budget = budget;
        record.version = record
            .version
            .checked_add(1)
            .expect("mandate version counter exhausted");
        for scope in record.scopes() {
            self.active_by_scope.entry(*scope).or_default().insert(id);
        }
    }

    pub(crate) fn revoke(&mut self, id: MandateId) {
        let (manager, scopes) = {
            let record = self
                .records
                .get(&id)
                .expect("validated mandate disappeared before revocation commit");
            (record.manager(), record.scopes().clone())
        };
        let removed = self.active_by_manager.remove(&manager);
        debug_assert_eq!(
            removed,
            Some(id),
            "Derived Data Consistency: active manager mandate index disagrees with record"
        );
        let removed_active = self.active.remove(&id);
        debug_assert!(
            removed_active,
            "Derived Data Consistency: revoked mandate was not indexed as active"
        );
        for scope in scopes {
            Self::remove_scope_index(&mut self.active_by_scope, scope, id);
        }
        let record = self
            .records
            .get_mut(&id)
            .expect("validated mandate disappeared before revocation commit");
        record.status = MandateStatus::Revoked;
        record.version = record
            .version
            .checked_add(1)
            .expect("mandate version counter exhausted");
    }

    fn remove_scope_index(
        index: &mut BTreeMap<ResponsibilityScope, BTreeSet<MandateId>>,
        scope: ResponsibilityScope,
        id: MandateId,
    ) {
        if let Some(ids) = index.get_mut(&scope) {
            ids.remove(&id);
            if ids.is_empty() {
                index.remove(&scope);
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            match record.status() {
                MandateStatus::Active => {
                    if self.active_by_manager.get(&record.manager()) != Some(&record.id())
                        || !self.active.contains(&record.id())
                    {
                        return false;
                    }
                    for scope in record.scopes() {
                        if !self
                            .active_by_scope
                            .get(scope)
                            .is_some_and(|ids| ids.contains(&record.id()))
                        {
                            return false;
                        }
                    }
                }
                MandateStatus::Revoked => {
                    if self.active_by_manager.get(&record.manager()) == Some(&record.id())
                        || self.active.contains(&record.id())
                    {
                        return false;
                    }
                    for scope in record.scopes() {
                        if self
                            .active_by_scope
                            .get(scope)
                            .is_some_and(|ids| ids.contains(&record.id()))
                        {
                            return false;
                        }
                    }
                }
            }
        }

        for (manager, id) in &self.active_by_manager {
            if !self.records.get(id).is_some_and(|record| {
                record.manager() == *manager && record.status() == MandateStatus::Active
            }) {
                return false;
            }
        }
        for (scope, ids) in &self.active_by_scope {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == MandateStatus::Active && record.scopes().contains(scope)
                }) {
                    return false;
                }
            }
        }
        for id in &self.active {
            if !self
                .records
                .get(id)
                .is_some_and(|record| record.status() == MandateStatus::Active)
            {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct MandateDraft {
    pub organization: OrganizationId,
    pub manager: CharacterId,
    pub scopes: BTreeSet<ResponsibilityScope>,
    pub standing_orders: BTreeMap<PolicyKind, PolicySetting>,
    pub budget: Option<BudgetAuthority>,
}

pub(crate) fn build_mandate_record(id: MandateId, draft: MandateDraft) -> MandateRecord {
    let MandateDraft {
        organization,
        manager,
        scopes,
        standing_orders,
        budget,
    } = draft;
    MandateRecord {
        id,
        organization,
        manager,
        scopes,
        standing_orders,
        budget,
        status: MandateStatus::Active,
        version: 1,
    }
}
