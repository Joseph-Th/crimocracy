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

/// One audience's current impression of one organization. Absent from the map means every
/// dimension sits at the authored baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationRecord {
    organization: OrganizationId,
    audience: AudienceKind,
    fear: u8,
    reliability: u8,
    competence: u8,
    treachery: u8,
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
            ReputationDimension::Fear => self.fear,
            ReputationDimension::Reliability => self.reliability,
            ReputationDimension::Competence => self.competence,
            ReputationDimension::Treachery => self.treachery,
        }
    }

    pub(crate) fn set_score(&mut self, dimension: ReputationDimension, value: u8) {
        match dimension {
            ReputationDimension::Fear => self.fear = value,
            ReputationDimension::Reliability => self.reliability = value,
            ReputationDimension::Competence => self.competence = value,
            ReputationDimension::Treachery => self.treachery = value,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReputationState {
    /// Keyed by (organization, audience); sparse — untouched impressions stay absent.
    records: BTreeMap<(OrganizationId, AudienceKind), ReputationRecord>,
    by_organization: BTreeMap<OrganizationId, Vec<AudienceKind>>,
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

    pub fn records_for_organization(
        &self,
        organization: OrganizationId,
    ) -> impl Iterator<Item = &ReputationRecord> {
        let audiences = self
            .by_organization
            .get(&organization)
            .cloned()
            .unwrap_or_default();
        audiences
            .into_iter()
            .filter_map(move |audience| self.records.get(&(organization, audience)))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &ReputationRecord> {
        self.records.values()
    }

    /// Inserts a first-touch reputation record. Records are created exactly once per
    /// (organization, audience) pair; later movement goes through `apply_delta`.
    pub(crate) fn insert_record(&mut self, record: ReputationRecord) {
        let key = (record.organization(), record.audience());
        self.by_organization
            .entry(record.organization())
            .or_default();
        if !self.by_organization[&record.organization()].contains(&record.audience()) {
            self.by_organization
                .get_mut(&record.organization())
                .expect("just-inserted organization bucket")
                .push(record.audience());
        }
        let previous = self.records.insert(key, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate reputation record inserted"
        );
    }

    /// Removes every record whose dimensions all sit at `baseline`. Decay erases fully faded
    /// impressions instead of pinning them at baseline forever, keeping "absent means
    /// unremarkable" literally true and bounding state growth.
    pub(crate) fn remove_at_baseline(&mut self, baseline: u8) {
        let faded: Vec<(OrganizationId, AudienceKind)> = self
            .records
            .iter()
            .filter(|(_, record)| {
                crate::reputation::ALL_REPUTATION_DIMENSIONS
                    .iter()
                    .all(|dimension| record.score(*dimension) == baseline)
            })
            .map(|(key, _)| *key)
            .collect();
        for key in faded {
            let (organization, audience) = key;
            self.records.remove(&key);
            if let Some(bucket) = self.by_organization.get_mut(&organization) {
                bucket.retain(|entry| *entry != audience);
                if bucket.is_empty() {
                    self.by_organization.remove(&organization);
                }
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for ((organization, audience), record) in &self.records {
            if *organization != record.organization()
                || *audience != record.audience()
                || !self
                    .by_organization
                    .get(organization)
                    .is_some_and(|bucket| bucket.contains(audience))
            {
                return false;
            }
        }
        for (organization, audiences) in &self.by_organization {
            for audience in audiences {
                if !self.records.contains_key(&(*organization, *audience)) {
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
            "Derived Data Consistency: reputation indexes disagree with source records"
        );
    }
}
