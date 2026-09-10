//! Contextual organizational reputation: per-audience standing across behavioral dimensions.
//!
//! Audiences hold separate impressions ([`GAME_DESIGN.md`] §26): the same event raises fear
//! among businesses while raising underworld respect. Records exist only where an audience's
//! impression has actually moved away from the authored baseline; absent entries mean
//! "unremarkable", so decay simply erases records rather than pinning every combination.
//!
//! One canonical mutation path lives in [`reputation_system`]; every producer — operation
//! consequences today, later negotiation, corruption, press behavior — applies typed deltas
//! through it. Consumers read resolved scores through [`reputation_system::resolve_score`].

pub mod reputation_system;

use crate::core::id::OrganizationId;
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AudienceKind {
    /// Other criminals: rivals, independent operators, potential recruits' circles.
    Underworld,
    /// Shopkeepers, venue owners, legitimate employers.
    Businesses,
    /// Neighborhood residents: witnesses, customers, community pressure.
    Residents,
    Police,
    Political,
    Press,
}

/// Behavioral axes an audience judges, deliberately distinct from character relationships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReputationDimension {
    /// Coercive weight: compliance extracted through anticipated consequences.
    Fear,
    /// Kept promises, predictable treatment of associates and payers.
    Reliability,
    /// Demonstrated effectiveness: jobs pulled off, rackets kept running.
    Competence,
    /// Suspected betrayal, broken deals, informants flipped.
    Treachery,
}

pub const ALL_REPUTATION_DIMENSIONS: [ReputationDimension; 4] = [
    ReputationDimension::Fear,
    ReputationDimension::Reliability,
    ReputationDimension::Competence,
    ReputationDimension::Treachery,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ReputationScore {
    value: u8,
    changed_at: SimTime,
}

impl ReputationScore {
    const fn at(value: u8, changed_at: SimTime) -> Self {
        Self { value, changed_at }
    }
}

/// One audience's current impression of one organization. Each dimension retains the time of
/// its latest real movement because daily decay must age the event that produced that specific
/// impression rather than weakening fresh standing merely because the campaign crossed midnight.
/// Absent from the map means every dimension sits at the authored baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationRecord {
    organization: OrganizationId,
    audience: AudienceKind,
    fear: ReputationScore,
    reliability: ReputationScore,
    competence: ReputationScore,
    treachery: ReputationScore,
}

impl ReputationRecord {
    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn audience(&self) -> AudienceKind {
        self.audience
    }

    pub fn score(&self, dimension: ReputationDimension) -> u8 {
        match dimension {
            ReputationDimension::Fear => self.fear.value,
            ReputationDimension::Reliability => self.reliability.value,
            ReputationDimension::Competence => self.competence.value,
            ReputationDimension::Treachery => self.treachery.value,
        }
    }

    pub(crate) fn changed_at(&self, dimension: ReputationDimension) -> SimTime {
        match dimension {
            ReputationDimension::Fear => self.fear.changed_at,
            ReputationDimension::Reliability => self.reliability.changed_at,
            ReputationDimension::Competence => self.competence.changed_at,
            ReputationDimension::Treachery => self.treachery.changed_at,
        }
    }

    pub(crate) fn set_score(
        &mut self,
        dimension: ReputationDimension,
        value: u8,
        changed_at: SimTime,
    ) {
        let score = ReputationScore::at(value, changed_at);
        match dimension {
            ReputationDimension::Fear => self.fear = score,
            ReputationDimension::Reliability => self.reliability = score,
            ReputationDimension::Competence => self.competence = score,
            ReputationDimension::Treachery => self.treachery = score,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReputationState {
    /// Keyed by (organization, audience); sparse — untouched impressions stay absent.
    records: BTreeMap<(OrganizationId, AudienceKind), ReputationRecord>,
}

impl ReputationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_record(
        &self,
        organization: OrganizationId,
        audience: AudienceKind,
    ) -> Option<&ReputationRecord> {
        self.records.get(&(organization, audience))
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &ReputationRecord> {
        self.records.values()
    }

    /// Inserts a first-touch reputation record. Records are created exactly once per
    /// (organization, audience) pair; later movement goes through `apply_delta`.
    pub(crate) fn insert_record(&mut self, record: ReputationRecord) {
        let key = (record.organization(), record.audience());
        let previous = self.records.insert(key, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate reputation record inserted"
        );
    }

    /// Removes one touched record when every dimension has returned to `baseline`. Canonical
    /// reputation mutation changes exactly one `(organization, audience)` record at a time, so
    /// rescanning the whole sparse map after every delta would make a day-boundary decay
    /// needlessly quadratic as a campaign accumulates audiences.
    pub(crate) fn remove_if_at_baseline(
        &mut self,
        key: (OrganizationId, AudienceKind),
        baseline: u8,
    ) {
        let is_neutral = self.records.get(&key).is_some_and(|record| {
            crate::reputation::ALL_REPUTATION_DIMENSIONS
                .iter()
                .all(|dimension| record.score(*dimension) == baseline)
        });
        if is_neutral {
            self.records.remove(&key);
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for ((organization, audience), record) in &self.records {
            if *organization != record.organization() || *audience != record.audience() {
                return false;
            }
        }
        true
    }
}
