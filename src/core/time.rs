//! Deterministic simulation time measured in campaign-relative minutes.

use serde::{Deserialize, Serialize};
use std::ops::Add;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SimTime(u64);

impl SimTime {
    pub const ZERO: Self = Self(0);

    pub const fn from_minutes(minutes: u64) -> Self {
        Self(minutes)
    }

    pub const fn as_minutes(self) -> u64 {
        self.0
    }

    /// Checked future scheduling. Canonical validators use this before persisting a due time so
    /// an otherwise valid state near the finite clock horizon returns a typed domain error
    /// instead of reaching the panicking `Add` convenience implementation.
    pub const fn checked_add(self, duration: SimDuration) -> Option<Self> {
        match self.0.checked_add(duration.0 as u64) {
            Some(minutes) => Some(Self(minutes)),
            None => None,
        }
    }
}

impl Add<SimDuration> for SimTime {
    type Output = Self;

    fn add(self, rhs: SimDuration) -> Self::Output {
        self.checked_add(rhs)
            .expect("simulation time overflowed u64 minutes")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimDuration(u32);

impl SimDuration {
    pub const ONE_MINUTE: Self = Self(1);

    pub const fn from_minutes(minutes: u32) -> Self {
        Self(minutes)
    }

    pub const fn as_minutes(self) -> u32 {
        self.0
    }
}

/// Freshness check for validate-then-commit tokens: `Err((expected, found))` when the
/// simulation clock is no longer exactly at the instant the token was validated at.
/// Callers map the pair into their own typed stale-time error.
pub fn ensure_time_current(now: SimTime, expected: SimTime) -> Result<(), (SimTime, SimTime)> {
    if now == expected {
        Ok(())
    } else {
        Err((expected, now))
    }
}

/// Length of one campaign day in minutes. Every daily pass (payroll, reputation decay,
/// autonomous recruitment, executive briefs, delegated enterprise expansion) keys its cadence
/// off this constant so the passes can never drift onto different boundaries.
pub const DAY_MINUTES: u64 = 1_440;

/// True exactly once per campaign day. Minute zero is never a boundary: state created at
/// the campaign start must not immediately run its daily passes.
pub fn is_day_boundary(now: SimTime) -> bool {
    let minutes = now.as_minutes();
    minutes != 0 && minutes.is_multiple_of(DAY_MINUTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_add_reports_simulation_clock_capacity() {
        assert_eq!(
            SimTime::from_minutes(u64::MAX - 4).checked_add(SimDuration::from_minutes(4)),
            Some(SimTime::from_minutes(u64::MAX))
        );
        assert_eq!(
            SimTime::from_minutes(u64::MAX - 4).checked_add(SimDuration::from_minutes(5)),
            None
        );
    }
}
