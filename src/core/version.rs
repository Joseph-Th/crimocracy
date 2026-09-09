//! Shared capacity guard for finite optimistic-concurrency version counters.

use thiserror::Error;

/// A versioned record has reached the end of its representable freshness space.
///
/// Versions must never wrap or saturate: either would let an old validated token become
/// indistinguishable from newer state. Canonical validators therefore reject one more mutation
/// before any authoritative write occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("{record_kind} version counter is exhausted")]
pub struct VersionCapacityError {
    record_kind: &'static str,
}

impl VersionCapacityError {
    pub const fn new(record_kind: &'static str) -> Self {
        Self { record_kind }
    }

    pub const fn record_kind(self) -> &'static str {
        self.record_kind
    }
}

pub(crate) fn ensure_version_can_advance(
    version: u32,
    record_kind: &'static str,
) -> Result<(), VersionCapacityError> {
    ensure_version_can_advance_by(version, 1, record_kind)
}

pub(crate) fn ensure_version_can_advance_by(
    version: u32,
    advances: u32,
    record_kind: &'static str,
) -> Result<(), VersionCapacityError> {
    if version.checked_add(advances).is_none() {
        Err(VersionCapacityError::new(record_kind))
    } else {
        Ok(())
    }
}

/// Advances a version only after the owning canonical path has preflighted capacity.
pub(crate) fn advance_version_preflighted(version: u32) -> u32 {
    version
        .checked_add(1)
        .expect("version capacity must be preflighted before authoritative mutation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_advance_accepts_last_available_version_slot() {
        ensure_version_can_advance(u32::MAX - 1, "test record")
            .expect("the last representable version advance should remain available");
        assert_eq!(advance_version_preflighted(u32::MAX - 1), u32::MAX);
    }

    #[test]
    fn single_advance_rejects_exhausted_version() {
        assert_eq!(
            ensure_version_can_advance(u32::MAX, "test record"),
            Err(VersionCapacityError::new("test record"))
        );
    }

    #[test]
    fn multi_advance_checks_the_whole_transaction_budget() {
        ensure_version_can_advance_by(u32::MAX - 2, 2, "test record")
            .expect("two remaining slots should support a two-write transaction");
        assert_eq!(
            ensure_version_can_advance_by(u32::MAX - 1, 2, "test record"),
            Err(VersionCapacityError::new("test record"))
        );
    }
}
