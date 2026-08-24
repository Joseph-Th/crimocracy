//! Directional character relationships; `relationship_system` is the sole mutation path.

pub mod relationship_system;

use crate::core::id::CharacterId;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RelationshipLevel(u8);

impl RelationshipLevel {
    pub fn try_new(value: u8) -> Result<Self, RelationshipLevelError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(RelationshipLevelError { value })
        }
    }
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for RelationshipLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("relationship level {value} is outside the inclusive range 0..=100")]
pub struct RelationshipLevelError {
    value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipDimensions {
    pub trust: RelationshipLevel,
    pub respect: RelationshipLevel,
    pub fear: RelationshipLevel,
    pub affection: RelationshipLevel,
    pub dependence: RelationshipLevel,
    pub resentment: RelationshipLevel,
    pub debt: RelationshipLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct RelationshipKey {
    from: CharacterId,
    to: CharacterId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelationshipRecord {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
    version: u32,
}

impl RelationshipRecord {
    pub fn from(&self) -> CharacterId {
        self.from
    }
    pub fn to(&self) -> CharacterId {
        self.to
    }
    pub fn dimensions(&self) -> RelationshipDimensions {
        self.dimensions
    }
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SocialState {
    relationships: BTreeMap<RelationshipKey, RelationshipRecord>,
    by_target: BTreeMap<CharacterId, BTreeSet<CharacterId>>,
}

impl SocialState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_relationship(
        &self,
        from: CharacterId,
        to: CharacterId,
    ) -> Option<&RelationshipRecord> {
        self.relationships.get(&RelationshipKey { from, to })
    }
    pub fn relationships_to(&self, to: CharacterId) -> impl Iterator<Item = &RelationshipRecord> {
        self.by_target
            .get(&to)
            .into_iter()
            .flatten()
            .filter_map(move |from| self.get_relationship(*from, to))
    }
    pub(crate) fn relationships(&self) -> impl Iterator<Item = &RelationshipRecord> {
        self.relationships.values()
    }
    pub(crate) fn upsert(
        &mut self,
        from: CharacterId,
        to: CharacterId,
        dimensions: RelationshipDimensions,
    ) {
        let key = RelationshipKey { from, to };
        match self.relationships.get_mut(&key) {
            Some(record) => {
                record.dimensions = dimensions;
                record.version = record
                    .version
                    .checked_add(1)
                    .expect("relationship version counter exhausted");
            }
            None => {
                self.relationships.insert(
                    key,
                    RelationshipRecord {
                        from,
                        to,
                        dimensions,
                        version: 1,
                    },
                );
                self.by_target.entry(to).or_default().insert(from);
            }
        }
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.relationships.values() {
            if !self
                .by_target
                .get(&record.to())
                .is_some_and(|sources| sources.contains(&record.from()))
            {
                return false;
            }
        }
        for (to, sources) in &self.by_target {
            for from in sources {
                if !self.relationships.contains_key(&RelationshipKey {
                    from: *from,
                    to: *to,
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
            "Derived Data Consistency: relationship indexes disagree with source records"
        );
    }
}
