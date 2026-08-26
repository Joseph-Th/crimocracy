//! Authored harness vocabulary: modes, strategies, scenarios, metrics, and aggregates.

use crimocracy::core::id::{
    BusinessId, CharacterId, EnterpriseId, FinancialAccountId, InformationId, MandateId,
    NeighborhoodId, OperationId, OpportunityId, OrganizationId,
};
use crimocracy::core::state::AppState;
use crimocracy::core::time::SimTime;
use crimocracy::finance::AccountKind;
use crimocracy::intelligence::{InformationTopic, Reliability, Specificity};
use crimocracy::operations::{
    OperationAbortCause, OperationAbortPhase, OperationKind, OperationObjectiveOutcome,
    ALL_OPERATION_KINDS,
};
use crimocracy::registry::Registry;
use crimocracy::social::RelationshipLevel;
use crimocracy::world::Rating;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const DEFAULT_BATCH_SAMPLES: u64 = 3;
pub const MAX_BATCH_SAMPLES: u64 = 64;
pub const DEFAULT_SEED: u64 = 0x1933_0514;
pub const MIN_SAMPLES_FOR_VARIATION_CONTRACT: u64 = 3;
/// Narrative comparisons rotate across this many adjacent seeds, covering every authored
/// fixture variation while matched branches inside one seed still share one world.
pub const NARRATIVE_SEED_ROTATION: u64 = 3;

/// Bounded deterministic policy choice for evaluation-owned variation. This is not a game
/// rule: it keeps controlled treatments from replaying one exact decision sequence forever
/// while staying fully determined by the run seed. The splitmix-style finalizer avalanches
/// all bits, so adjacent seeds diverge in every choice rather than only in high bits.
pub fn bounded_policy_choice(seed: u64, salt: u64, choices: u64) -> u64 {
    if choices == 0 {
        return 0;
    }
    let mut mixed = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed ^= mixed >> 27;
    mixed = mixed.wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^= mixed >> 31;
    mixed % choices
}

/// Longest authored operation duration plus a fixed margin, so the terminal-wait guard in
/// [`crate::session`] tracks authored content instead of a constant that could go stale as
/// authors add longer operations.
pub fn operation_wait_slack_minutes(registry: &Registry) -> u32 {
    ALL_OPERATION_KINDS
        .iter()
        .map(|kind| {
            registry
                .get_operation(*kind)
                .execution()
                .duration()
                .as_minutes()
        })
        .max()
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessMode {
    Smoke,
    Full,
}

impl HarnessMode {
    pub fn parse(value: &str) -> Result<Self, HarnessCliError> {
        match value {
            "smoke" => Ok(Self::Smoke),
            "full" => Ok(Self::Full),
            _ => Err(HarnessCliError::InvalidMode {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessOptions {
    pub mode: HarnessMode,
    pub samples: u64,
    pub seed: u64,
    pub strategy: Option<Strategy>,
    pub artifact_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessCliError {
    #[error("{flag} requires a value")]
    MissingValue { flag: &'static str },
    #[error("invalid {flag} value '{value}'")]
    InvalidValue { flag: &'static str, value: String },
    #[error("unsupported gameplay_harness argument '{argument}'")]
    UnsupportedArgument { argument: String },
    #[error("unsupported harness mode '{value}'; expected 'smoke' or 'full'")]
    InvalidMode { value: String },
    #[error("--samples must be between 1 and {MAX_BATCH_SAMPLES}, found {value}")]
    SampleCountOutOfRange { value: u64 },
    #[error("smoke mode accepts only --samples 1, found {value}")]
    SmokeSampleCount { value: u64 },
    #[error("unsupported --strategy value '{value}'; expected 'all', 'rush', 'press', or 'recon'")]
    InvalidStrategy { value: String },
    #[error("--strategy is only supported in smoke mode")]
    StrategyOnlyInSmoke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Rush,
    Press,
    Recon,
}

impl Strategy {
    pub fn parse(value: &str) -> Result<Option<Self>, HarnessCliError> {
        match value {
            "all" => Ok(None),
            "rush" => Ok(Some(Self::Rush)),
            "press" => Ok(Some(Self::Press)),
            "recon" => Ok(Some(Self::Recon)),
            _ => Err(HarnessCliError::InvalidStrategy {
                value: value.to_owned(),
            }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rush => "RUSH",
            Self::Press => "PRESS",
            Self::Recon => "RECON",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessContractError {
    #[error("harness run did not record its strategy")]
    MissingStrategy,
    #[error("{strategy:?} run did not authorize a burglary")]
    MissingBurglary { strategy: Strategy },
    #[error("{strategy:?} run did not reach a terminal burglary state")]
    MissingTerminalState { strategy: Strategy },
    #[error(
        "{strategy:?} run has inconsistent terminal state: aborted={aborted}, outcome={outcome:?}, abort_phase={abort_phase:?}, abort_cause={abort_cause:?}"
    )]
    InconsistentTerminalState {
        strategy: Strategy,
        aborted: bool,
        outcome: Option<OperationObjectiveOutcome>,
        abort_phase: Option<OperationAbortPhase>,
        abort_cause: Option<OperationAbortCause>,
    },
    #[error("{strategy:?} run did not complete its financial observation window")]
    MissingFinancialObservation { strategy: Strategy },
    #[error("{strategy:?} night-trap run did not expose expected evidence: {evidence}")]
    MissingStrategyEvidence {
        strategy: Strategy,
        evidence: &'static str,
    },
    #[error("{strategy:?} run never reached its matched financial boundary minute")]
    MissingMatchedFinancialSnapshot { strategy: Strategy },
    #[error(
        "cross-branch matched-window financial contract violated: legitimate {legitimate:?}; enterprise {enterprise:?}"
    )]
    FinancialBranchMismatch {
        legitimate: [Option<i64>; 3],
        enterprise: [Option<i64>; 3],
    },
    #[error(
        "operation {operation:?} did not reach terminal state between minute {started_at} and guard deadline {deadline}"
    )]
    OperationDidNotTerminate {
        operation: OperationId,
        started_at: u64,
        deadline: u64,
    },
    #[error(
        "{profile:?} batch observed only {observed} fixture variation(s); expected at least {required}"
    )]
    InsufficientFixtureVariation {
        profile: ScenarioProfile,
        observed: usize,
        required: usize,
    },
    #[error("surveillance report did not contain actionable recurring patrol windows")]
    NoActionablePatrolWindows,
    #[error("no safe operation window was derivable from the surveillance report")]
    NoSafeOperationWindow,
    #[error(
        "{strategy:?} run recorded inconsistent laundering evidence: gross {gross}c minus fee {fee}c does not equal the accounted balance {balance:?}c"
    )]
    InconsistentLaunderingEvidence {
        strategy: Strategy,
        gross: i64,
        fee: i64,
        balance: Option<i64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioProfile {
    NightTrap,
    LatePatrol,
    VeteranCrew,
    ThinCrew,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
                "A delivery clerk directly observed the Bellmore service annex receiving high-value consignments after midnight."
            }
            Self::Crowded => {
                "A delivery clerk directly observed Calder's receiving house storing high-value consignments after midnight."
            }
            Self::Quiet => {
                "A delivery clerk directly observed the Vesper annex storing high-value consignments after midnight."
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
                "Bellmore Jewelry closes with valuable stock still on site; the rear service access may be workable."
            }
            Self::Crowded => {
                "Calder's Jewelers closes with valuable stock still on site; a loading-bay access may be workable."
            }
            Self::Quiet => {
                "Vesper Gold closes with valuable stock still on site; a side-street access may be workable."
            }
        }
    }

    pub fn source_summary(self) -> &'static str {
        match self {
            Self::Clockwork => {
                "Lena Orr says Bellmore keeps valuable stock overnight and uses a rear service entrance; she does not know the alarm or patrol pattern."
            }
            Self::Crowded => {
                "Lena Orr says Calder's keeps valuable stock overnight and uses a loading bay; she does not know the alarm or patrol pattern."
            }
            Self::Quiet => {
                "Lena Orr says Vesper keeps valuable stock overnight and uses a side street; she does not know the alarm or patrol pattern."
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
            (ScenarioProfile::LatePatrol, Self::Clockwork) => [(180, 120, 90), (1_320, 120, 70)],
            (ScenarioProfile::LatePatrol, Self::Crowded) => [(240, 120, 84), (1_260, 150, 76)],
            (ScenarioProfile::LatePatrol, Self::Quiet) => [(300, 120, 76), (1_200, 150, 64)],
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew,
                Self::Clockwork,
            ) => [(120, 120, 90), (1_320, 120, 70)],
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew,
                Self::Crowded,
            ) => [(90, 150, 84), (1_260, 150, 76)],
            (
                ScenarioProfile::NightTrap
                | ScenarioProfile::VeteranCrew
                | ScenarioProfile::ThinCrew,
                Self::Quiet,
            ) => [(60, 180, 76), (1_200, 150, 64)],
        }
    }
}

impl ScenarioProfile {
    pub const SENSITIVITY_SET: [Self; 3] = [Self::LatePatrol, Self::VeteranCrew, Self::ThinCrew];

    pub fn label(self) -> &'static str {
        match self {
            Self::NightTrap => "NIGHT TRAP",
            Self::LatePatrol => "LATE PATROL",
            Self::VeteranCrew => "VETERAN CREW",
            Self::ThinCrew => "THIN CREW",
        }
    }

    pub fn lieutenant_management(self) -> u8 {
        match self {
            Self::VeteranCrew => 95,
            Self::ThinCrew => 60,
            Self::NightTrap | Self::LatePatrol => 78,
        }
    }

    pub fn burglar_burglary(self) -> u8 {
        match self {
            Self::VeteranCrew => 96,
            Self::ThinCrew => 62,
            Self::NightTrap | Self::LatePatrol => 82,
        }
    }

    pub fn burglar_stealth(self) -> u8 {
        match self {
            Self::VeteranCrew => 92,
            Self::ThinCrew => 58,
            Self::NightTrap | Self::LatePatrol => 76,
        }
    }

    pub fn scout_surveillance(self) -> u8 {
        match self {
            Self::VeteranCrew => 94,
            Self::ThinCrew => 72,
            Self::NightTrap | Self::LatePatrol => 90,
        }
    }

    pub fn scout_stealth(self) -> u8 {
        match self {
            Self::VeteranCrew => 92,
            Self::ThinCrew => 66,
            Self::NightTrap | Self::LatePatrol => 84,
        }
    }
}

/// Evaluation-owned policy timing anchored to authored runtime values. These are not additional
/// game rules: they describe when this controlled treatment chooses to act. Matched strategy
/// branches receive the same timeline, while the seed-derived offsets keep batch runs from
/// replaying one exact clock sequence forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScenarioTimeline {
    pub initial_burglary_at: SimTime,
    pub initial_opportunity_valid_until: SimTime,
    pub second_opportunity_discovery_at: SimTime,
    pub second_opportunity_valid_until: SimTime,
    pub rush_second_act_at: SimTime,
    pub recon_second_act_surveillance_at: SimTime,
}

impl ScenarioTimeline {
    pub fn for_scenario(registry: &Registry, seed: u64) -> Self {
        let campaign_day_minutes = u64::from(
            registry
                .recruitment()
                .autonomous_attempt_cadence()
                .as_minutes(),
        );
        let burglary_duration = u64::from(
            registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration()
                .as_minutes(),
        );
        let policy_variant = (seed / 3) % 5;
        let initial_burglary_at = 120 + 10 * (seed % 5);
        let initial_opportunity_window =
            (campaign_day_minutes / 2).max(burglary_duration.saturating_add(60));
        let second_opportunity_discovery_at = campaign_day_minutes
            .saturating_add(10)
            .saturating_add(policy_variant * 10);
        let second_opportunity_window =
            (campaign_day_minutes / 2).max(burglary_duration.saturating_add(60));
        let second_opportunity_valid_until =
            second_opportunity_discovery_at.saturating_add(second_opportunity_window);
        let rush_second_act_at = second_opportunity_discovery_at
            .saturating_add(campaign_day_minutes * 3 / 8)
            .saturating_add(policy_variant * 10);
        let recon_second_act_surveillance_at = second_opportunity_discovery_at
            .saturating_add(50)
            .saturating_add(policy_variant * 5);

        Self {
            initial_burglary_at: SimTime::from_minutes(initial_burglary_at),
            initial_opportunity_valid_until: SimTime::from_minutes(initial_opportunity_window),
            second_opportunity_discovery_at: SimTime::from_minutes(second_opportunity_discovery_at),
            second_opportunity_valid_until: SimTime::from_minutes(second_opportunity_valid_until),
            rush_second_act_at: SimTime::from_minutes(rush_second_act_at),
            recon_second_act_surveillance_at: SimTime::from_minutes(
                recon_second_act_surveillance_at,
            ),
        }
    }
}

pub struct Scenario<'registry> {
    pub registry: &'registry Registry,
    pub state: AppState,
    pub player: OrganizationId,
    pub rival: OrganizationId,
    pub second_rival: OrganizationId,
    pub police: OrganizationId,
    pub neighborhood: NeighborhoodId,
    pub target: BusinessId,
    pub alternate_target: BusinessId,
    pub front: BusinessId,
    pub resale_venue: BusinessId,
    pub liquidation_cash: FinancialAccountId,
    pub liquidation_settlement: FinancialAccountId,
    /// Organization accounted funds: the clean-money side of laundering through an owned
    /// cash-intensive front.
    pub accounted_funds: FinancialAccountId,
    pub boss: CharacterId,
    pub lieutenant: CharacterId,
    pub burglar: CharacterId,
    pub scout: CharacterId,
    /// The primary target's owner: the on-scene witness a witnessed exposure names on the
    /// case, and the person an intimidated witness-pressure operation would lean on.
    pub target_owner: CharacterId,
    /// Player-side replacement candidate for act 2: an independent with a pre-existing personal
    /// relationship to the boss, recruitable only through canonical executive recruitment.
    pub danny_ferro: CharacterId,
    pub detective: CharacterId,
    /// The organization's standing Police-channel institutional contact: the boss's personal
    /// line to the precinct's lead detective, established through the canonical contact path.
    pub police_contact: crimocracy::core::id::ContactId,
    pub opportunity_information: InformationId,
    pub alternate_opportunity_information: InformationId,
    pub enterprise: EnterpriseId,
    /// Second-district expansion fixture: a quiet neighborhood the organization can diversify
    /// into while its home district is legally hot. Unused by RUSH/RECON; PRESS capitalizes it.
    pub expansion_neighborhood: NeighborhoodId,
    pub expansion_front: BusinessId,
    pub expansion_cash: FinancialAccountId,
    pub expansion_settlement: FinancialAccountId,
    /// Rival-held fixture assets the delegated rival-expansion pass governs: a home-district
    /// venue plus the Rosetti treasury accounts it draws cash and settlement from.
    pub rival_venue: BusinessId,
    pub rival_cash: FinancialAccountId,
    pub rival_settlement: FinancialAccountId,
    pub lieutenant_mandate: MandateId,
    pub investigation: Option<crimocracy::core::id::InvestigationId>,
    pub variation: FixtureVariation,
    pub timeline: ScenarioTimeline,
    /// The run seed this scenario was built from: evaluation-owned policy variation (timing
    /// offsets, approach choices, watch order) derives from it, never from hidden game state.
    pub seed: u64,
    /// Registry-derived slack for the terminal-wait guard: the longest authored operation
    /// duration, so the guard tracks content instead of a hard-coded constant.
    pub wait_slack_minutes: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RunMetrics {
    pub strategy: Option<Strategy>,
    pub variation: Option<FixtureVariation>,
    pub burglary: Option<OperationId>,
    pub outcome: Option<OperationObjectiveOutcome>,
    pub aborted: bool,
    pub abort_phase: Option<OperationAbortPhase>,
    pub abort_cause: Option<OperationAbortCause>,
    pub police_dispatched: bool,
    pub police_arrived: bool,
    pub decision_requests: u32,
    pub player_police_activity_information: u32,
    pub planning_information_count: usize,
    pub planning_information_topics: BTreeSet<InformationTopic>,
    pub counterintelligence_outcome: Option<OperationObjectiveOutcome>,
    pub counterintelligence_information: usize,
    pub followup_case_active: Option<bool>,
    pub cold_case_confirmed: Option<bool>,
    pub case_cold_minute: Option<u64>,
    pub case_open_minute: Option<u64>,
    pub counterintelligence_scheduled_at: Option<u64>,
    pub exposure_score: Option<i16>,
    pub exposure_level: Option<crimocracy::operations::OperationExposureLevel>,
    pub investigation_created: bool,
    /// True once any operation-originated case was staffed during the session, whatever
    /// operation it came from. District street heat keys off every active operation-originated
    /// case, so branch-heating contracts must use this session-wide signal rather than the
    /// burglary's own resolution record.
    pub session_case_staffed: bool,
    /// Whether the branch's own act-2 surveillance drew a police case: self-inflicted heat the
    /// organization only knows about through its surfaced after-action report.
    pub self_heat_case_opened: bool,
    /// What the organization's player-visible channel read about that self-inflicted case
    /// before the window closed: Some(true) still active, Some(false) shelved, None no read.
    pub self_heat_case_active: Option<bool>,
    pub evidence_count: usize,
    pub investigation_work_scheduled: u32,
    pub investigation_work_resolved: u32,
    pub burglary_information_quality: Option<u8>,
    pub property_acquired_value_cents: Option<i64>,
    pub property_realized_cash_cents: Option<i64>,
    pub burglary_terminal_minute: Option<u64>,
    pub liquidation_minute: Option<u64>,
    pub legitimate_net_cents: Option<i64>,
    pub enterprise_net_cents: Option<i64>,
    /// Cumulative finances snapshotted at the shared campaign-day boundary before any
    /// branch-specific arc extension, so cross-branch comparisons stay window-honest.
    pub matched_financial_boundary_minute: Option<u64>,
    pub matched_legitimate_net_cents: Option<i64>,
    pub matched_enterprise_net_cents: Option<i64>,
    pub discovered_surveillance_information: usize,
    pub player_legal_activity_information: usize,
    pub player_report_count: usize,
    pub executive_brief_count: usize,
    /// Raw audit evidence of delegated rival growth: total active rackets non-player
    /// organizations operate in the home district at session end, derived through the
    /// canonical territory-influence surface. Never read by acting policy.
    pub rival_home_enterprises: u32,
    pub autonomous_recruitment_attempts: u32,
    pub player_personnel_departures: u32,
    /// Refused autonomous poaching approaches against this organization's members. Each one
    /// produced a player-visible loyalty report naming the outside recruiter.
    pub player_poach_warnings: u32,
    pub defector: Option<crimocracy::core::id::CharacterId>,
    pub defection_minute: Option<u64>,
    pub defector_trail_confirmed: Option<bool>,
    /// Win-back evidence: once the trail confirmed where a defector landed, leadership made one
    /// canonical executive re-approach through the recruitment path.
    pub win_back_attempted: bool,
    /// Whether that re-approach was accepted; `None` when no attempt was made.
    pub win_back_accepted: Option<bool>,
    /// The authored margin the win-back attempt resolved at, quoted from the attempt record.
    pub win_back_margin: Option<i16>,
    /// On refusal, production rules deliver a loyalty report to the recruiting organization
    /// naming our recruiter: reaching out carries an intelligence cost. `None` when not refused.
    pub win_back_refusal_leaked_to_rival: Option<bool>,
    // Act-2 (second wind) evidence: the narrative branches either rebuild and recover value on a
    // reopened second score or deliberately let it lapse as the price of standing down.
    pub second_opportunity: Option<OpportunityId>,
    pub second_opportunity_discovered: bool,
    pub second_opportunity_expired: bool,
    pub replacement: Option<crimocracy::core::id::CharacterId>,
    pub replacement_recruited: bool,
    pub second_burglary: Option<OperationId>,
    pub second_burglary_aborted: bool,
    pub second_burglary_outcome: Option<OperationObjectiveOutcome>,
    pub second_burglary_terminal_minute: Option<u64>,
    pub second_act_recon_information: usize,
    pub second_act_property_acquired_value_cents: Option<i64>,
    pub second_act_property_realized_cash_cents: Option<i64>,
    /// Debrief knowledge after a standing abort: the district-scoped PoliceActivity record
    /// the organization holds as an abort artifact, carried into second-score planning.
    pub debrief_patrol_information: Vec<InformationId>,
    /// Topics carried into the second-score plan, proving what act 2 actually knew.
    pub second_act_planning_topics: BTreeSet<InformationTopic>,
    // District-diversification evidence: PRESS buys its second-district venue outright
    // through the canonical acquisition path (gated on accounted funds), then capitalizes
    // a second-district enterprise out of idle street cash while the home district is
    // legally hot.
    pub expansion_enterprise: Option<EnterpriseId>,
    pub expansion_established: bool,
    /// Whether this session ran in the primary narrative comparison set. The strict
    /// legitimate-wealth demonstration is anchored there so every full run proves the
    /// chain deterministically; rotated sets accept whatever ending their authored
    /// economy honestly produced.
    pub primary_narrative_set: bool,
    /// Whether this session's racket float is authored as concealed cash. Concealed money
    /// cannot route through a front's ledgers, so a concealed-till PRESS world legitimately
    /// ends its standing-down wait in survival rather than clean-money diversification.
    pub enterprise_till_concealed: Option<bool>,
    pub expansion_net_cents: Option<i64>,
    /// True once the branch purchased the harbor venue through the canonical acquisition
    /// path: ownership moved, and accounted funds paid the authored price in full.
    pub front_acquired: bool,
    /// The authored kind price the acquisition actually paid, quoted from production state.
    pub acquisition_price_cents: Option<i64>,
    /// Accounted funds spent on acquisitions; part of the clean-money accounting identity.
    pub acquisition_spent_cents: i64,
    /// Validated purchase attempts that failed for short books before the price was covered:
    /// the player-visible shape of the legitimacy gate on dirty money.
    pub acquisition_rejections: u32,
    /// Canonical police-contact channel usage: how many times the organization asked its
    /// standing contact what the institution knows and received a fresh disclosure.
    pub contact_reads: u32,
    /// Player-owned cycles that drew a vice inquiry: sustained district casework converting
    /// into a dedicated investigation on the racket itself.
    pub vice_inquiries_drawn: u32,
    // Witness-chain evidence: a witnessed exposure on a character-owned business names the
    // owner as the case's witness, interviews are scheduled institutionally, and the player
    // can answer with canonical witness pressure.
    /// The case named its on-scene witness at intake (audit fact; the organization only
    /// knows the job "was witnessed" from its own after-action).
    pub case_witness_registered: bool,
    /// Institutional witness interviews scheduled against the session's case (audit count).
    pub witness_interviews_scheduled: u32,
    /// Whether an interview connected into recorded witness testimony on the case (audit).
    pub witness_testimony_produced: bool,
    /// Whether leadership ran the canonical WitnessPressure operation against the witness.
    pub witness_pressure_attempted: bool,
    pub witness_pressure_outcome: Option<OperationObjectiveOutcome>,
    /// True when the pressure op aborted under its police-arrival contingency before any
    /// resolution: the discipline shape of the counter-play.
    pub witness_pressure_aborted: bool,
    /// Audit fact: the pressure degraded the witness's registered cooperation.
    pub witness_cooperation_degraded: bool,
    /// Autonomous evidence-threshold arrests of this organization's members during the
    /// session. Custody is the production consequence chain: representation, referral.
    pub player_member_arrests: u32,
    // Money-state evidence: street cash routed through an owned cash-intensive front's books.
    pub laundered_gross_cents: i64,
    /// The front's authored cut of everything it absorbed.
    pub launder_fee_cents: i64,
    /// Times the books refused a request because it exceeded the cycle's plausible volume:
    /// the player-visible shape of the laundering constraint.
    pub laundering_capacity_rejections: u32,
    /// Final accounted-funds balance, for the clean-money accounting identity contract.
    pub accounted_balance_cents: Option<i64>,
    /// Payroll evidence across the session: what wages cost and where they went unpaid.
    pub payroll_paid_cents: i64,
    pub payroll_short_cents: i64,
}

#[derive(Default)]
pub struct Aggregate {
    pub samples: u64,
    pub fixture_variations: BTreeSet<FixtureVariation>,
    pub achieved: u64,
    pub partial: u64,
    pub failed: u64,
    pub aborted: u64,
    pub unresolved: u64,
    pub police_dispatched: u64,
    pub police_arrived: u64,
    pub decisions: u64,
    pub investigations: u64,
    pub investigation_work_scheduled: u64,
    pub investigation_work_resolved: u64,
    pub exposure_total: i64,
    pub exposure_samples: u64,
    pub intelligence_total: u64,
    pub intelligence_samples: u64,
    pub property_acquired_total_cents: i128,
    pub property_realized_total_cents: i128,
    pub burglary_terminal_minute_total: u128,
    pub burglary_terminal_samples: u64,
    pub liquidation_minute_total: u128,
    pub liquidation_samples: u64,
    pub standing_contingency_aborts: u64,
    pub legal_activity_information_sessions: u64,
    pub police_activity_information_sessions: u64,
    pub followup_case_active_sessions: u64,
    pub cold_case_confirmed_sessions: u64,
    pub player_report_total: u64,
    pub executive_brief_total: u64,
    pub autonomous_recruitment_attempts: u64,
    pub player_personnel_departures: u64,
    pub player_poach_warnings: u64,
    pub contact_reads: u64,
    pub vice_inquiries: u64,
    pub witness_cases: u64,
    pub witness_pressure_attempts: u64,
    pub witness_testimony_sessions: u64,
    pub player_member_arrests: u64,
    pub rival_home_enterprises_total: u64,
    pub front_acquisitions: u64,
    pub laundered_gross_total_cents: i128,
    pub accounted_balance_total_cents: i128,
    pub accounted_balance_samples: u64,
    pub payroll_paid_total_cents: i128,
    pub payroll_short_total_cents: i128,
}

impl Aggregate {
    pub fn add(&mut self, metrics: &RunMetrics) {
        self.samples += 1;
        if let Some(variation) = metrics.variation {
            self.fixture_variations.insert(variation);
        }
        match metrics.outcome {
            Some(OperationObjectiveOutcome::Achieved) => self.achieved += 1,
            Some(OperationObjectiveOutcome::Partial) => self.partial += 1,
            Some(OperationObjectiveOutcome::Failed) => self.failed += 1,
            None if metrics.aborted => self.aborted += 1,
            // A run that reached neither a resolution nor an abort is an unresolved breakage; count
            // it explicitly so the outcome percentages never silently stop summing to the sample
            // count.
            None => self.unresolved += 1,
        }
        self.police_dispatched += u64::from(metrics.police_dispatched);
        self.police_arrived += u64::from(metrics.police_arrived);
        self.decisions += u64::from(metrics.decision_requests);
        self.investigations += u64::from(metrics.investigation_created);
        self.investigation_work_scheduled += u64::from(metrics.investigation_work_scheduled);
        self.investigation_work_resolved += u64::from(metrics.investigation_work_resolved);
        if let Some(score) = metrics.exposure_score {
            self.exposure_total += i64::from(score);
            self.exposure_samples += 1;
        }
        if let Some(quality) = metrics.burglary_information_quality {
            self.intelligence_total += u64::from(quality);
            self.intelligence_samples += 1;
        }
        if let Some(value) = metrics.property_acquired_value_cents {
            self.property_acquired_total_cents += i128::from(value);
        }
        if let Some(value) = metrics.property_realized_cash_cents {
            self.property_realized_total_cents += i128::from(value);
        }
        if let Some(minute) = metrics.burglary_terminal_minute {
            self.burglary_terminal_minute_total += u128::from(minute);
            self.burglary_terminal_samples += 1;
        }
        if let Some(minute) = metrics.liquidation_minute {
            self.liquidation_minute_total += u128::from(minute);
            self.liquidation_samples += 1;
        }
        if matches!(
            metrics.abort_cause,
            Some(OperationAbortCause::PoliceArrival(_))
        ) {
            self.standing_contingency_aborts += 1;
        }
        self.legal_activity_information_sessions +=
            u64::from(metrics.player_legal_activity_information > 0);
        self.police_activity_information_sessions +=
            u64::from(metrics.player_police_activity_information > 0);
        self.followup_case_active_sessions += u64::from(metrics.followup_case_active == Some(true));
        self.cold_case_confirmed_sessions += u64::from(metrics.cold_case_confirmed == Some(true));
        self.player_report_total += metrics.player_report_count as u64;
        self.executive_brief_total += metrics.executive_brief_count as u64;
        self.autonomous_recruitment_attempts += u64::from(metrics.autonomous_recruitment_attempts);
        self.player_personnel_departures += u64::from(metrics.player_personnel_departures);
        self.player_poach_warnings += u64::from(metrics.player_poach_warnings);
        self.contact_reads += u64::from(metrics.contact_reads);
        self.vice_inquiries += u64::from(metrics.vice_inquiries_drawn);
        self.witness_cases += u64::from(metrics.case_witness_registered);
        self.witness_pressure_attempts += u64::from(metrics.witness_pressure_attempted);
        self.witness_testimony_sessions += u64::from(metrics.witness_testimony_produced);
        self.player_member_arrests += u64::from(metrics.player_member_arrests);
        self.rival_home_enterprises_total += u64::from(metrics.rival_home_enterprises);
        self.front_acquisitions += u64::from(metrics.front_acquired);
        self.laundered_gross_total_cents += i128::from(metrics.laundered_gross_cents);
        if let Some(balance) = metrics.accounted_balance_cents {
            self.accounted_balance_total_cents += i128::from(balance);
            self.accounted_balance_samples += 1;
        }
        self.payroll_paid_total_cents += i128::from(metrics.payroll_paid_cents);
        self.payroll_short_total_cents += i128::from(metrics.payroll_short_cents);
    }

    pub fn percent(&self, value: u64) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            (value as f64 * 100.0) / self.samples as f64
        }
    }

    pub fn print(&self, label: &str) {
        let avg_exposure = if self.exposure_samples == 0 {
            0.0
        } else {
            self.exposure_total as f64 / self.exposure_samples as f64
        };
        let avg_intelligence = if self.intelligence_samples == 0 {
            0.0
        } else {
            self.intelligence_total as f64 / self.intelligence_samples as f64
        };
        let avg_acquired_property = if self.samples == 0 {
            0.0
        } else {
            self.property_acquired_total_cents as f64 / self.samples as f64
        };
        let avg_realized_property = if self.samples == 0 {
            0.0
        } else {
            self.property_realized_total_cents as f64 / self.samples as f64
        };
        let avg_terminal_minute = if self.burglary_terminal_samples == 0 {
            0.0
        } else {
            self.burglary_terminal_minute_total as f64 / self.burglary_terminal_samples as f64
        };
        let avg_liquidation_minute = if self.liquidation_samples == 0 {
            0.0
        } else {
            self.liquidation_minute_total as f64 / self.liquidation_samples as f64
        };
        // Block layout instead of one wide line: outcomes and legal pressure on the first band,
        // intelligence and economics on the second, so a strategy row stays scannable.
        println!(
            "{label:<6} samples {:>2}  fixtures {:?}
       outcomes: achieved {:>5.1}%  partial {:>5.1}%  failed {:>5.1}%  aborted {:>5.1}%  unresolved {:>2}
       pressure: standing aborts {:>5.1}%  police arrivals {:>5.1}%  staffed cases {:>5.1}%  case work {}/{}
                 surfaced decisions {}  legal intel {:>5.1}%  police intel {:>5.1}%  case hot {:>5.1}%  case cold {:>5.1}%
       economy:  avg exposure {:>5.1}  avg intel {:>5.1}  avg finish {:>5.0}m  avg property {:>8.0}c -> {:>8.0}c cash @ {:>5.0}m
       rhythm:   reports {:>3}  briefs {:>3}  rival attempts {:>3}  poach warnings {:>3}  departures {:>3}  contact reads {:>3}  vice hits {:>3}
                 payroll paid {:>7.0}c  unpaid {:>6.0}c
       witness:  named cases {}  pressure runs {}  testimony sessions {}  member arrests {}  rival rackets {:>4.1}  acquisitions {}
       money:    laundered {:>8.0}c gross  accounted balance {:>7.0}c",
            self.samples,
            self.fixture_variations,
            self.percent(self.achieved),
            self.percent(self.partial),
            self.percent(self.failed),
            self.percent(self.aborted),
            self.unresolved,
            self.percent(self.standing_contingency_aborts),
            self.percent(self.police_arrived),
            self.percent(self.investigations),
            self.investigation_work_scheduled,
            self.investigation_work_resolved,
            self.decisions,
            self.percent(self.legal_activity_information_sessions),
            self.percent(self.police_activity_information_sessions),
            self.percent(self.followup_case_active_sessions),
            self.percent(self.cold_case_confirmed_sessions),
            avg_exposure,
            avg_intelligence,
            avg_terminal_minute,
            avg_acquired_property,
            avg_realized_property,
            avg_liquidation_minute,
            self.player_report_total,
            self.executive_brief_total,
            self.autonomous_recruitment_attempts,
            self.player_poach_warnings,
            self.player_personnel_departures,
            self.contact_reads,
            self.vice_inquiries,
            self.payroll_paid_total_cents as f64 / self.samples as f64,
            self.payroll_short_total_cents as f64 / self.samples as f64,
            self.witness_cases,
            self.witness_pressure_attempts,
            self.witness_testimony_sessions,
            self.player_member_arrests,
            self.rival_home_enterprises_total as f64 / self.samples as f64,
            self.front_acquisitions,
            self.laundered_gross_total_cents as f64 / self.samples as f64,
            if self.accounted_balance_samples == 0 {
                0.0
            } else {
                self.accounted_balance_total_cents as f64 / self.accounted_balance_samples as f64
            },
        );
    }
}

#[derive(Clone)]
pub struct FinancialView {
    pub legitimate_cycle_count: u32,
    pub legitimate_net_cents: i64,
    pub enterprise_cycle_count: usize,
    pub enterprise_net_cents: i64,
    /// Per-enterprise economics so the readout distinguishes the canal book from any
    /// diversification the branch opened during its arc.
    pub enterprise_lines: Vec<EnterpriseLine>,
    pub liquidation_cash_cents: i64,
    pub held_property_operations: u32,
    pub held_property_value_cents: i64,
    pub liquidated_property_operations: u32,
    pub liquidated_property_cash_cents: i64,
    /// Organization-owned balances grouped by account kind, so the readout shows the cash
    /// position a boss would actually govern, not only cycle flows.
    pub cash_position: Vec<(AccountKind, i64)>,
    /// Money-state evidence: what the organization routed through its front's books.
    pub laundered_gross_cents: i64,
    pub launder_fee_cents: i64,
    pub laundering_capacity_rejections: u32,
    /// Session-to-date wage costs from observed payroll outcomes.
    pub payroll_paid_cents: i64,
    pub payroll_short_cents: i64,
}

#[derive(Clone)]
pub struct EnterpriseLine {
    pub label: String,
    pub cycle_count: usize,
    pub net_cents: i64,
    pub cash_cents: i64,
}

pub fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("gameplay harness ratings are authored within 0..=100")
}

pub fn jitter_rating_u8(base: u8, jitter: i16) -> u8 {
    (i16::from(base) + jitter).clamp(0, 100) as u8
}

pub fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value)
        .expect("gameplay harness relationship levels are authored within 0..=100")
}
