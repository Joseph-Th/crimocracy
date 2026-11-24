//! Persistent, contested municipal campaigning. Political standing is purchased with
//! accounted funds, erodes daily, and must be defended against rival organizations.
//! Municipal elections are certified by the daily political phase; campaign victory is claimed separately.

use crate::core::id::{CharacterId, FinancialAccountId, NeighborhoodId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod political_system;

pub const INFLUENCE_CAP: u16 = 1_000;
pub const MAYORAL_INFLUENCE_REQUIRED: u16 = 800;
pub const INFLUENCE_COST_PER_POINT: i64 = 5_000;
pub const MAYORAL_CAMPAIGN_SPEND_REQUIRED: i64 = 35_000_000;
pub const MAYORAL_LIQUID_RESERVE_REQUIRED: i64 = 15_000_000;
pub const MAYORAL_SUSTAINED_DAYS: u64 = 90;
pub const MUNICIPAL_ELECTION_INTERVAL_DAYS: u64 = 180;
pub const MAYORAL_FILING_LEAD_DAYS: u64 = 30;
pub const MAYORAL_FILING_FEE_CENTS: i64 = 2_000_000;
pub const GRASSROOTS_REQUIRED: u16 = 300;
pub const INSTITUTIONAL_REQUIRED: u16 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CivicInitiative {
    PublicMeetings,
    NeighborhoodServices,
    LaborCoalition,
    LocalPress,
}

impl CivicInitiative {
    pub const fn cost_cents(self) -> i64 {
        match self {
            Self::PublicMeetings => 150_000,
            Self::NeighborhoodServices => 450_000,
            Self::LaborCoalition => 350_000,
            Self::LocalPress => 300_000,
        }
    }

    pub const fn boosts(self) -> (u16, u16, u16) {
        // Organizational influence, grassroots voters, institutional support.
        match self {
            Self::PublicMeetings => (10, 70, 0),
            Self::NeighborhoodServices => (5, 160, 0),
            Self::LaborCoalition => (0, 15, 120),
            Self::LocalPress => (15, 50, 60),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MayoralCandidacy {
    pub candidate: CharacterId,
    pub filed_at: SimTime,
    pub election_day: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardBallot {
    pub district: NeighborhoodId,
    pub candidate_votes: u32,
    pub incumbent_votes: u32,
    pub rival_votes: u32,
    pub ward_won: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MayoralElection {
    pub candidate: CharacterId,
    /// An actual political officeholder, if a municipal council has one.
    pub incumbent: Option<CharacterId>,
    pub election_day: u64,
    pub held_at: SimTime,
    pub wards_needed: usize,
    pub wards_won: usize,
    pub eligible: bool,
    pub elected: bool,
    pub ballots: Vec<WardBallot>,
}

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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictCampaign {
    pub(crate) influence: u16,
    pub(crate) grassroots: u16,
    pub(crate) institutions: u16,
    pub(crate) leading_since: Option<SimTime>,
    pub(crate) specialists: BTreeMap<SpecialistKind, CharacterId>,
    pub(crate) initiatives: BTreeMap<CivicInitiative, SimTime>,
}

impl DistrictCampaign {
    pub fn influence(&self) -> u16 {
        self.influence
    }

    pub fn grassroots(&self) -> u16 {
        self.grassroots
    }

    pub fn institutions(&self) -> u16 {
        self.institutions
    }

    pub fn initiative_last_held(&self, kind: CivicInitiative) -> Option<SimTime> {
        self.initiatives.get(&kind).copied()
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
    pub(crate) candidacy: Option<MayoralCandidacy>,
    pub(crate) elections: Vec<MayoralElection>,
}

impl OrganizationCampaign {
    pub fn spent_cents(&self) -> i64 {
        self.spent_cents
    }

    pub fn district(&self, id: NeighborhoodId) -> Option<&DistrictCampaign> {
        self.districts.get(&id)
    }

    pub(crate) fn district_campaigns(&self) -> impl Iterator<Item = &DistrictCampaign> {
        self.districts.values()
    }

    pub fn candidacy(&self) -> Option<MayoralCandidacy> {
        self.candidacy
    }

    pub fn latest_election(&self) -> Option<&MayoralElection> {
        self.elections.last()
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
mod integration_tests;
#[cfg(test)]
mod tests;
