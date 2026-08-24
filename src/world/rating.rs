//! Bounded 0..=100 rating values and their qualitative presentation bands, shared by
//! capabilities, drives, and derived scores across subsystems.

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Rating(u8);

impl Rating {
    pub const MAX: u8 = 100;

    pub fn try_new(value: u8) -> Result<Self, RatingError> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(RatingError { value })
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    pub const fn qualitative_band(self) -> QualitativeBand {
        match self.0 {
            0..=19 => QualitativeBand::Poor,
            20..=44 => QualitativeBand::Competent,
            45..=69 => QualitativeBand::Skilled,
            70..=89 => QualitativeBand::Excellent,
            90..=100 => QualitativeBand::Exceptional,
            _ => unreachable!(),
        }
    }
}

impl<'de> Deserialize<'de> for Rating {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("rating {value} is outside the inclusive range 0..=100")]
pub struct RatingError {
    value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualitativeBand {
    Poor,
    Competent,
    Skilled,
    Excellent,
    Exceptional,
}
