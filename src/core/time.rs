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
}

impl Add<SimDuration> for SimTime {
    type Output = Self;

    fn add(self, rhs: SimDuration) -> Self::Output {
        Self(
            self.0
                .checked_add(u64::from(rhs.0))
                .expect("simulation time overflowed u64 minutes"),
        )
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
