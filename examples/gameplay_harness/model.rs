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
/// Minutes of slack added to each operation's authored duration to form the terminal-wait guard,
/// staying registry-derived instead of a hard-coded window that could go stale as authors add
/// longer operations. The guard uses the per-operation duration plus this slack, which is sized
/// to cover police arrival variance and decision deferral for the longest authored operation.
pub const OPERATION_WAIT_SLACK_MINUTES: u32 = 240;
/// Small deterministic margin past the authored cold-case shelf instant so the narrative re-check
/// observes a case the simulation has already shelved, without depending on in-tick scheduling.
pub const COLD_CASE_RECHECK_SLACK_MINUTES: u32 = 10;

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
    pub boss: CharacterId,
    pub lieutenant: CharacterId,
    pub burglar: CharacterId,
    pub scout: CharacterId,
    /// Player-side replacement candidate for act 2: an independent with a pre-existing personal
    /// relationship to the boss, recruitable only through canonical executive recruitment.
    pub danny_ferro: CharacterId,
    pub detective: CharacterId,
    pub opportunity_information: InformationId,
    pub alternate_opportunity_information: InformationId,
    pub enterprise: EnterpriseId,
    /// Second-district expansion fixture: a quiet neighborhood the organization can diversify
    /// into while its home district is legally hot. Unused by RUSH/RECON; PRESS capitalizes it.
    pub expansion_neighborhood: NeighborhoodId,
    pub expansion_front: BusinessId,
    pub expansion_cash: FinancialAccountId,
    pub expansion_settlement: FinancialAccountId,
    pub lieutenant_mandate: MandateId,
    pub investigation: Option<crimocracy::core::id::InvestigationId>,
    pub variation: FixtureVariation,
    pub timeline: ScenarioTimeline,
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
    pub autonomous_recruitment_attempts: u32,
    pub player_personnel_departures: u32,
    /// Refused autonomous poaching approaches against this organization's members. Each one
    /// produced a player-visible loyalty report naming the outside recruiter.
    pub player_poach_warnings: u32,
    pub defector: Option<crimocracy::core::id::CharacterId>,
    pub defection_minute: Option<u64>,
    pub defector_trail_confirmed: Option<bool>,
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
    // District-diversification evidence: PRESS capitalizes a second-district enterprise out of
    // its idle street cash while the home district is legally hot, so the same heat that taxes
    // the canal racket never touches the harbor book.
    pub expansion_enterprise: Option<EnterpriseId>,
    pub expansion_established: bool,
    pub expansion_net_cents: Option<i64>,
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
       pressure: standing aborts {:>5.1}%  police arrivals {:>5.1}%  cases opened {:>5.1}%  case work {}/{}
                 surfaced decisions {}  legal intel {:>5.1}%  police intel {:>5.1}%  case hot {:>5.1}%  case cold {:>5.1}%
       economy:  avg exposure {:>5.1}  avg intel {:>5.1}  avg finish {:>5.0}m  avg property {:>8.0}c -> {:>8.0}c cash @ {:>5.0}m
       rhythm:   reports {:>3}  briefs {:>3}  rival attempts {:>3}  poach warnings {:>3}  departures {:>3}",
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
