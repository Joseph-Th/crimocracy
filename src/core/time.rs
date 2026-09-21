//! Deterministic simulation time measured in campaign-relative minutes.

use serde::{Deserialize, Serialize};
use std::ops::Add;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SimTime(u64);

impl SimTime {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

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

/// Length of one campaign day in minutes in the two widths required by durable time and
/// minute-of-day records, plus the matching duration used by authored daily schedulers. Keep
/// the literal here so patrol, intelligence, budgets, and daily systems cannot drift onto
/// different definitions of a simulation day.
pub const DAY_MINUTES_U16: u16 = 1_440;
pub const DAY_MINUTES: u64 = DAY_MINUTES_U16 as u64;
pub const DAY_DURATION: SimDuration = SimDuration::from_minutes(DAY_MINUTES_U16 as u32);

/// True exactly on a positive multiple of the supplied cadence. Minute zero is never a
/// recurring boundary: state created at campaign start must not immediately run periodic work.
/// A zero cadence is treated as disabled rather than reaching `is_multiple_of(0)`.
pub fn is_recurring_boundary(now: SimTime, cadence: SimDuration) -> bool {
    let cadence_minutes = u64::from(cadence.as_minutes());
    now != SimTime::ZERO && cadence_minutes != 0 && now.as_minutes().is_multiple_of(cadence_minutes)
}

/// True exactly once per campaign day. Minute zero is never a boundary: state created at
/// the campaign start must not immediately run its daily passes.
pub fn is_day_boundary(now: SimTime) -> bool {
    is_recurring_boundary(now, DAY_DURATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_boundary_excludes_campaign_start_and_zero_cadence() {
        assert!(!is_recurring_boundary(SimTime::ZERO, DAY_DURATION));
        assert!(!is_recurring_boundary(
            SimTime::from_minutes(DAY_MINUTES),
            SimDuration::from_minutes(0),
        ));
        assert!(is_recurring_boundary(
            SimTime::from_minutes(DAY_MINUTES),
            DAY_DURATION,
        ));
    }

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
