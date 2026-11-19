//! Authored opening variation, timing, and seed-derived content.
//!
//! Scenario construction belongs in `scenario.rs`.

use crimocracy::core::time::{DAY_MINUTES, SimTime};
use crimocracy::intelligence::{Reliability, Specificity};
use crimocracy::operations::OperationKind;
use crimocracy::registry::Registry;

/// Explicit seed channels for a campaign. `world` determines fixture variation
/// and the simulation RNG streams; `policy` determines authored timing variation.
/// Keeping them in one typed value prevents a sensitivity comparison from
/// accidentally perturbing both at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationSeeds {
    pub world: u64,
    pub policy: u64,
}

impl EvaluationSeeds {
    pub const fn new(world: u64, policy: u64) -> Self {
        Self { world, policy }
    }
}

/// The authored opening a session can boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioProfile {
    NightTrap,
    LatePatrol,
    FleetingWindow,
    VeteranCrew,
    ThinCrew,
    GreenScout,
}

/// Deterministic variation in the authored opening, derived from the world seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum FixtureVariation {
    Clockwork,
    Crowded,
    Quiet,
}

impl FixtureVariation {
    pub fn from_seed(seed: u64) -> Self {
        match seed % 3 {
            0 => Self::Clockwork,
            1 => Self::Crowded,
            2 => Self::Quiet,
            _ => unreachable!("seed remainder modulo three is bounded"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clockwork => "CLOCKWORK",
            Self::Crowded => "CROWDED",
            Self::Quiet => "QUIET",
        }
    }

    pub fn neighborhood_name(self) -> &'static str {
        match self {
            Self::Clockwork => "South Ward",
            Self::Crowded => "Market Row",
            Self::Quiet => "Canal District",
        }
    }

    pub fn target_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Bellmore Jewelry",
            Self::Crowded => "Calder's Jewelers",
            Self::Quiet => "Vesper Gold",
        }
    }

    pub fn alternate_target_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Bellmore Service Annex",
            Self::Crowded => "Calder's Receiving House",
            Self::Quiet => "Vesper Gold Annex",
        }
    }

    pub fn alternate_source_summary(self) -> &'static str {
        match self {
            Self::Clockwork => {
                "A delivery clerk directly observed the Bellmore service annex taking fresh consignments in after midnight."
            }
            Self::Crowded => {
                "A delivery clerk directly observed Calder's receiving house taking fresh consignments in after midnight."
            }
            Self::Quiet => {
                "A delivery clerk directly observed the Vesper annex taking fresh consignments in after midnight."
            }
        }
    }

    pub fn front_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Fulton Social Club",
            Self::Crowded => "Lantern Room",
            Self::Quiet => "Marlowe Club",
        }
    }

    pub fn resale_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Mercer Pawn & Exchange",
            Self::Crowded => "Redline Exchange",
            Self::Quiet => "Northline Exchange",
        }
    }

    pub fn rival_venue_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Grotto Card Room",
            Self::Crowded => "Vera's Back Room",
            Self::Quiet => "Sable Social Club",
        }
    }

    pub fn opportunity_summary(self) -> &'static str {
        match self {
            Self::Clockwork => {
                "Bellmore Jewelry closes with stock still on site; the rear service access may be workable."
            }
            Self::Crowded => {
                "Calder's Jewelers closes with stock still on site; a loading-bay access may be workable."
            }
            Self::Quiet => {
                "Vesper Gold closes with stock still on site; a side-street access may be workable."
            }
        }
    }

    pub fn source_summary(self) -> &'static str {
        match self {
            Self::Clockwork => {
                "Lena Orr says Bellmore keeps its stock overnight and uses a rear service entrance; she does not know the alarm or patrol pattern."
            }
            Self::Crowded => {
                "Lena Orr says Calder's keeps its stock overnight and uses a loading bay; she does not know the alarm or patrol pattern."
            }
            Self::Quiet => {
                "Lena Orr says Vesper keeps its stock overnight and uses a side street; she does not know the alarm or patrol pattern."
            }
        }
    }

    pub fn source_reliability(self) -> Reliability {
        match self {
            Self::Clockwork => Reliability::Mixed,
            Self::Crowded => Reliability::GenerallyReliable,
            Self::Quiet => Reliability::GenerallyReliable,
        }
    }

    pub fn source_specificity(self) -> Specificity {
        match self {
            Self::Clockwork => Specificity::General,
            Self::Crowded => Specificity::Specific,
            Self::Quiet => Specificity::General,
        }
    }

    pub fn neighborhood_police_presence(self) -> u8 {
        match self {
            Self::Clockwork => 58,
            Self::Crowded => 72,
            Self::Quiet => 42,
        }
    }

    pub fn neighborhood_economy(self) -> (u8, u8, u8) {
        match self {
            Self::Clockwork => (62, 78, 72),
            Self::Crowded => (74, 91, 84),
            Self::Quiet => (48, 61, 56),
        }
    }

    pub fn patrol_windows(self, profile: ScenarioProfile) -> [(u16, u16, u8); 2] {
        match (profile, self) {
            (ScenarioProfile::LatePatrol | ScenarioProfile::FleetingWindow, Self::Clockwork) => {
                [(180, 120, 90), (1_320, 120, 70)]
            }
            (ScenarioProfile::LatePatrol | ScenarioProfile::FleetingWindow, Self::Crowded) => {
                [(240, 120, 84), (1_260, 150, 76)]
            }
            (ScenarioProfile::LatePatrol | ScenarioProfile::FleetingWindow, Self::Quiet) => {
                [(300, 120, 76), (1_200, 150, 64)]
            }
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew
                | ScenarioProfile::GreenScout,
                Self::Clockwork,
            ) => [(120, 120, 90), (1_320, 120, 70)],
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew
                | ScenarioProfile::GreenScout,
                Self::Crowded,
            ) => [(90, 150, 84), (1_260, 150, 76)],
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew
                | ScenarioProfile::GreenScout,
                Self::Quiet,
            ) => [(60, 180, 76), (1_200, 150, 64)],
        }
    }
}

impl ScenarioProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::NightTrap => "NIGHT TRAP",
            Self::LatePatrol => "LATE PATROL",
            Self::FleetingWindow => "FLEETING WINDOW",
            Self::VeteranCrew => "VETERAN CREW",
            Self::ThinCrew => "THIN CREW",
            Self::GreenScout => "GREEN SCOUT",
        }
    }

    /// Ambient institutional pressure for the controlled scenario. LatePatrol and FleetingWindow
    /// deliberately keep the background low so their named treatments isolate patrol timing and
    /// opportunity-window cost instead of being overwhelmed by ordinary district policing.
    /// The main NightTrap and crew-quality profiles retain each fixture's natural ambient level.
    pub fn neighborhood_police_presence(self, variation: FixtureVariation) -> u8 {
        match self {
            // LatePatrol is a residual-risk control: ordinary policing can still catch an
            // immediate move in the most exposed district, while learned timing and planning
            // information can reduce dispatch pressure.
            Self::LatePatrol => match variation {
                FixtureVariation::Clockwork => 28,
                FixtureVariation::Crowded => 43,
                FixtureVariation::Quiet => 22,
            },
            // FleetingWindow isolates the time cost of scouting. Keep police pressure below the
            // treatment so losing the opportunity can be attributed to elapsed time, not heat.
            Self::FleetingWindow => match variation {
                FixtureVariation::Clockwork => 10,
                FixtureVariation::Crowded => 12,
                FixtureVariation::Quiet => 8,
            },
            Self::NightTrap | Self::VeteranCrew | Self::ThinCrew | Self::GreenScout => {
                variation.neighborhood_police_presence()
            }
        }
    }

    pub fn lieutenant_management(self) -> u8 {
        match self {
            Self::VeteranCrew => 95,
            Self::ThinCrew => 60,
            Self::NightTrap | Self::LatePatrol | Self::FleetingWindow | Self::GreenScout => 78,
        }
    }

    pub fn burglar_burglary(self) -> u8 {
        match self {
            Self::VeteranCrew => 96,
            Self::ThinCrew => 62,
            Self::NightTrap | Self::LatePatrol | Self::FleetingWindow | Self::GreenScout => 82,
        }
    }

    pub fn burglar_stealth(self) -> u8 {
        match self {
            Self::VeteranCrew => 92,
            Self::ThinCrew => 58,
            Self::NightTrap | Self::LatePatrol | Self::FleetingWindow | Self::GreenScout => 76,
        }
    }

    pub fn scout_surveillance(self) -> u8 {
        match self {
            Self::VeteranCrew => 94,
            Self::ThinCrew => 72,
            Self::GreenScout => 58,
            Self::NightTrap | Self::LatePatrol | Self::FleetingWindow => 90,
        }
    }

    pub fn scout_stealth(self) -> u8 {
        match self {
            Self::VeteranCrew => 92,
            Self::ThinCrew => 66,
            Self::GreenScout => 52,
            Self::NightTrap | Self::LatePatrol | Self::FleetingWindow => 84,
        }
    }
}

/// Deadline for the initial burglary opportunity.
pub fn initial_opportunity_deadline(
    registry: &Registry,
    policy_seed: u64,
    profile: ScenarioProfile,
) -> SimTime {
    let duration = u64::from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .duration()
            .as_minutes(),
    );
    let minutes = if profile == ScenarioProfile::FleetingWindow {
        10 + 5 * (policy_seed % 3) + duration + 15
    } else {
        (DAY_MINUTES / 2).max(duration + 60)
    };
    SimTime::from_minutes(minutes)
}

/// Shared opening street-treasury seed. The financial view excludes this from
/// earned flow so the readout shows what the organization produced, not its
/// starting stake.
pub(crate) const STARTING_TREASURY_CENTS: i64 = 80_000;

/// Authored rating, clamped to the range the content is written for.
pub fn rating(value: u8) -> crimocracy::world::Rating {
    crimocracy::world::Rating::try_new(value)
        .expect("authored campaign ratings are written within 0..=100")
}

/// Offset an authored rating and clamp it back into range.
pub fn jitter_rating_u8(base: u8, jitter: i16) -> u8 {
    (i16::from(base) + jitter).clamp(0, 100) as u8
}

/// Authored relationship level, clamped to the range the content is written for.
pub fn level(value: u8) -> crimocracy::social::RelationshipLevel {
    crimocracy::social::RelationshipLevel::try_new(value)
        .expect("authored campaign relationship levels are written within 0..=100")
}

/// Parse a seed flag value, accepting `0x[hex]` or all-decimal digits. Anything
/// else is a typo and is rejected rather than silently reinterpreted in another
/// radix.
pub fn parse_seed_value(flag: &'static str, raw: &str) -> Result<u64, String> {
    let invalid = || format!("{flag} expects a decimal or 0x-prefixed hex value, got {raw:?}");
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| invalid())
    } else if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) {
        raw.parse::<u64>().map_err(|_| invalid())
    } else {
        Err(invalid())
    }
}
