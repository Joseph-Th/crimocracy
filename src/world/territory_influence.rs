//! Read-only per-district economic-influence aggregation over canonical world, enterprise
//! records; no mutation paths live here.
//!
//! Territory answers "who holds sway in this district" from record-backed economic presence:
//! active enterprises operated at district locations, regardless of host ownership. Every
//! figure is derived from authoritative records at resolution time; nothing about influence
//! is persisted.
//!
//! The consumer is simulation-side delegated expansion (`autonomous_expansion` resolves each
//! district's unique strict leader before consolidating). This summary is not a player
//! information feed: surfacing it verbatim would bypass the provenance-bearing intelligence
//! model.

use crate::core::id::{NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TerritoryInfluenceError {
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
}

/// One organization's record-backed footprint inside a single district.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerritoryStanding {
    pub organization: OrganizationId,
    /// Active enterprises operating at district locations, regardless of host ownership.
    pub active_enterprises: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborhoodInfluenceSummary {
    pub neighborhood: NeighborhoodId,
    /// Ascending organization order; organizations with no district footprint are absent.
    pub standings: Vec<TerritoryStanding>,
}

impl NeighborhoodInfluenceSummary {
    /// The unique strict leader on active-enterprise count, if any. Contested districts —
    /// ties between two or more organizations — have no leader, because territory is
    /// contested rather than shared by default.
    pub fn economic_leader(&self) -> Option<OrganizationId> {
        let mut leader: Option<(u32, OrganizationId)> = None;
        let mut tied = false;
        for standing in &self.standings {
            let count = standing.active_enterprises;
            if count == 0 {
                continue;
            }
            match leader {
                None => {
                    leader = Some((count, standing.organization));
                    tied = false;
                }
                Some((best_count, _)) if best_count < count => {
                    leader = Some((count, standing.organization));
                    tied = false;
                }
                Some((best_count, _)) if best_count == count => tied = true,
                Some(_) => {}
            }
        }
        if tied {
            None
        } else {
            leader.map(|(_, organization)| organization)
        }
    }
}

/// Resolves who holds sway in a district from canonical records alone. Deterministic:
/// standings ascend by organization id.
pub fn resolve_neighborhood_influence(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Result<NeighborhoodInfluenceSummary, TerritoryInfluenceError> {
    if state.world().get_neighborhood(neighborhood).is_none() {
        return Err(TerritoryInfluenceError::MissingNeighborhood(neighborhood));
    }

    // Economic presence: active enterprises whose hosted or district location resolves to
    // this neighborhood. The location index serves district-hosted rackets directly and the
    // neighborhood's own businesses cover venue-hosted ones, without scanning every racket
    // in the city. Ordered scans keep the derivation deterministic.
    let mut enterprises_by_org: BTreeMap<OrganizationId, u32> = BTreeMap::new();
    let neighborhood_enterprises = state
        .enterprises()
        .enterprises_at(EnterpriseLocation::Neighborhood(neighborhood))
        .filter(|record| record.status() == EnterpriseStatus::Active)
        .map(|record| record.organization())
        .chain(
            state
                .world()
                .businesses_in_neighborhood(neighborhood)
                .flat_map(|business| {
                    state
                        .enterprises()
                        .enterprises_at(EnterpriseLocation::Business(business.id()))
                })
                .filter(|record| record.status() == EnterpriseStatus::Active)
                .map(|record| record.organization()),
        );
    for organization in neighborhood_enterprises {
        *enterprises_by_org.entry(organization).or_default() += 1;
    }

    let standings = enterprises_by_org
        .into_iter()
        .map(|(organization, active_enterprises)| TerritoryStanding {
            organization,
            active_enterprises,
        })
        .collect();

    Ok(NeighborhoodInfluenceSummary {
        neighborhood,
        standings,
    })
}

#[cfg(test)]
mod tests;
