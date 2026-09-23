//! Custody release timing and validated release transactions.

use super::ArrestError;
use crate::core::id::ArrestId;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::ensure_version_can_advance;
use crate::legal::ArrestStatus;

/// The latest instant custody may remain active. The simulation clock is finite, so an authored
/// detention window extending beyond it ends at the last representable minute rather than
/// becoming an immortal detention because `SimTime + duration` overflowed.
pub(crate) fn custody_release_at(arrested_at: SimTime, maximum_detention: SimDuration) -> SimTime {
    arrested_at
        .checked_add(maximum_detention)
        .unwrap_or(SimTime::MAX)
}

#[derive(Debug)]
pub struct ValidatedRelease {
    arrest: ArrestId,
    expected_version: u32,
}

impl ValidatedRelease {
    pub(crate) fn arrest(&self) -> ArrestId {
        self.arrest
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), ArrestError> {
        let record = state
            .legal
            .get_arrest(self.arrest)
            .ok_or(ArrestError::MissingArrest(self.arrest))?;
        if record.version() != self.expected_version {
            return Err(ArrestError::StaleArrest {
                arrest: self.arrest,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != ArrestStatus::Detained {
            return Err(ArrestError::NotDetained(self.arrest));
        }
        ensure_version_can_advance(record.version(), "arrest")?;
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), ArrestError> {
        self.ensure_current(state)?;
        state.legal.release_arrest(self.arrest, state.now());
        Ok(())
    }
}

pub fn validate_release_arrest(
    state: &AppState,
    arrest: ArrestId,
) -> Result<ValidatedRelease, ArrestError> {
    let record = state
        .legal
        .get_arrest(arrest)
        .ok_or(ArrestError::MissingArrest(arrest))?;
    if record.status() != ArrestStatus::Detained {
        return Err(ArrestError::NotDetained(arrest));
    }
    ensure_version_can_advance(record.version(), "arrest")?;
    Ok(ValidatedRelease {
        arrest,
        expected_version: record.version(),
    })
}

/// Releases detainees whose modeled custody window has elapsed. Prosecution referral and review
/// can continue after release, but charging, bail, trial, and sentence custody remain outside the
/// current legal foundation, so an arrest cannot imply permanent confinement merely because no
/// modeled court-custody layer exists to advance it. The authored window is long enough for the
/// detainee informant decision to occur first.
pub(crate) fn apply_due_custody_releases(
    state: &mut AppState,
    maximum_detention: SimDuration,
) -> Result<Vec<ArrestId>, ArrestError> {
    let now = state.now();
    let mut due: Vec<ArrestId> = if now == SimTime::MAX {
        state
            .legal
            .detained_arrests()
            .map(|arrest| arrest.id())
            .collect()
    } else if let Some(cutoff_minutes) = now
        .as_minutes()
        .checked_sub(u64::from(maximum_detention.as_minutes()))
    {
        state
            .legal
            .detained_arrest_ids_arrested_on_or_before(SimTime::from_minutes(cutoff_minutes))
            .collect()
    } else {
        Vec::new()
    };
    // The chronology index groups by arrest time. Preserve the custody pass's canonical
    // arrest-id order so this optimization cannot alter observable tick ordering.
    due.sort_unstable();
    let releases = due
        .iter()
        .copied()
        .map(|arrest| validate_release_arrest(state, arrest))
        .collect::<Result<Vec<_>, _>>()?;
    for release in releases {
        // Every due record was prevalidated before the first release. Release touches only its
        // own arrest record, so no earlier release can stale a later token in this cohort.
        release
            .commit(state)
            .expect("prevalidated due custody release must remain current");
    }
    Ok(due)
}
