//! Arrest and custody record vocabulary plus derived custody indexes.

use crate::core::id::{ArrestId, CharacterId, EvidenceId, InvestigationId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArrestStatus {
    Detained,
    Released,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArrestRecord {
    pub(in crate::legal) id: ArrestId,
    pub(in crate::legal) character: CharacterId,
    pub(in crate::legal) authority: OrganizationId,
    pub(in crate::legal) investigation: InvestigationId,
    pub(in crate::legal) evidence: BTreeSet<EvidenceId>,
    pub(in crate::legal) arrested_at: SimTime,
    pub(in crate::legal) released_at: Option<SimTime>,
    pub(in crate::legal) status: ArrestStatus,
    pub(in crate::legal) version: u32,
}

impl ArrestRecord {
    pub fn id(&self) -> ArrestId {
        self.id
    }

    pub fn character(&self) -> CharacterId {
        self.character
    }

    pub fn authority(&self) -> OrganizationId {
        self.authority
    }

    pub fn investigation(&self) -> InvestigationId {
        self.investigation
    }

    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }

    pub fn arrested_at(&self) -> SimTime {
        self.arrested_at
    }

    pub fn released_at(&self) -> Option<SimTime> {
        self.released_at
    }

    pub fn status(&self) -> ArrestStatus {
        self.status
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug)]
pub struct ArrestDraft {
    pub character: CharacterId,
    pub investigation: InvestigationId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct ArrestIndexes {
    pub(in crate::legal) by_investigation: BTreeMap<InvestigationId, BTreeSet<ArrestId>>,
    pub(in crate::legal) active_by_character: BTreeMap<CharacterId, ArrestId>,
    /// Every currently detained arrest, so custody-wide consumers such as automatic legal
    /// support scan live detainees instead of the full arrest history.
    pub(in crate::legal) detained: BTreeSet<ArrestId>,
    /// Currently detained arrests grouped by their immutable custody start. Release and
    /// one-shot informant timing can therefore select only the relevant chronology slice
    /// instead of rescanning every live detainee each minute.
    pub(in crate::legal) detained_by_arrested_at: BTreeMap<SimTime, BTreeSet<ArrestId>>,
}
