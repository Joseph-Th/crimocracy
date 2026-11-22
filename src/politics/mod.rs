//! Persistent, contested municipal campaigning. Political standing is purchased with
//! accounted funds, erodes daily, and must be defended against rival organizations.
//! The election is decided by the campaign-victory owner, not by an autonomous tick.

use crate::core::id::{CharacterId, FinancialAccountId, NeighborhoodId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod political_system;

pub const INFLUENCE_CAP: u16 = 1_000;
pub const MAYORAL_INFLUENCE_REQUIRED: u16 = 800;
pub const INFLUENCE_COST_PER_POINT: i64 = 5_000;
pub const MAYORAL_CAMPAIGN_SPEND_REQUIRED: i64 = 10_000_000;
pub const MAYORAL_LIQUID_RESERVE_REQUIRED: i64 = 5_000_000;
pub const MAYORAL_SUSTAINED_DAYS: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SpecialistKind {
    Organizer,
    Publicist,
    Strategist,
}

impl SpecialistKind {
    pub const fn signing_cost(self) -> i64 {
        match self {
            Self::Organizer => 150_000,
            Self::Publicist => 350_000,
            Self::Strategist => 600_000,
        }
    }

    pub const fn daily_salary(self) -> i64 {
        match self {
            Self::Organizer => 3_000,
            Self::Publicist => 8_000,
            Self::Strategist => 16_000,
        }
    }

    pub const fn daily_growth(self) -> u16 {
        match self {
            Self::Organizer => 8,
            Self::Publicist => 18,
            Self::Strategist => 34,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictCampaign {
    pub(crate) influence: u16,
    pub(crate) leading_since: Option<SimTime>,
    pub(crate) specialists: BTreeMap<SpecialistKind, CharacterId>,
}

impl DistrictCampaign {
    pub fn influence(&self) -> u16 {
        self.influence
    }

    pub fn leading_since(&self) -> Option<SimTime> {
        self.leading_since
    }

    pub fn specialists(&self) -> &BTreeMap<SpecialistKind, CharacterId> {
        &self.specialists
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationCampaign {
    pub(crate) spent_cents: i64,
    pub(crate) clearing_account: Option<FinancialAccountId>,
    pub(crate) districts: BTreeMap<NeighborhoodId, DistrictCampaign>,
}

impl OrganizationCampaign {
    pub fn spent_cents(&self) -> i64 {
        self.spent_cents
    }

    pub fn district(&self, id: NeighborhoodId) -> Option<&DistrictCampaign> {
        self.districts.get(&id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticsState {
    pub(crate) organizations: BTreeMap<OrganizationId, OrganizationCampaign>,
}

impl PoliticsState {
    pub fn organization(&self, id: OrganizationId) -> Option<&OrganizationCampaign> {
        self.organizations.get(&id)
    }
}

#[cfg(test)]
mod tests;
