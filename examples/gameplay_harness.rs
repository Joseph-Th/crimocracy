//! Controlled/calibration harness for deterministic strategy and integration evidence.
//!
//! RUSH, PRESS, and RECON use canonical production operations and player-visible information.
//! `[DEV AUDIT]` output may inspect hidden state after decisions for diagnostics, but hidden state
//! never feeds action selection. `[NARRATION]` lines are the harness's documentary voice: they
//! explain world causality from player-visible facts and never feed action selection either.
//! Narrative sessions also run a player-earned defector watch after an accepted defection: the
//! organization watches every known rival through canonical surveillance and confirms where the
//! departed member resurfaces, instead of the departure report leaking the recruiting organization.
//! Timeline anchors are derived from the authored registry (operation duration, autonomous
//! recruitment cadence, and cold-case window) so session timing tracks the game instead of a
//! second hard-coded ruleset. Batch runs vary the simulation seed; each seed rotates the fixture
//! and bounded policy timing while matched branches stay on the same scenario timeline.

use crimocracy::build_registry;
use crimocracy::core::attention::AttentionClass;
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{
    BusinessId, CharacterId, EnterpriseId, InformationId, OperationId, OpportunityId,
    OrganizationId,
};
use crimocracy::core::invariants::{validate_state, validate_state_against_registry};
use crimocracy::core::simulation::{run_tick, TickOutcome};
use crimocracy::core::state::AppState;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::decisions::{DecisionContext, DecisionResponse, OperationExceptionReason};
use crimocracy::delegation::delegation_system::MandateRevisionDraft;
use crimocracy::delegation::delegation_system::{validate_assign_mandate, validate_revise_mandate};
use crimocracy::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crimocracy::economy::business_economy_system::validate_establish_business_economy;
use crimocracy::economy::business_reporting::resolve_organization_business_financial_summary;
use crimocracy::economy::BusinessEconomyDraft;
use crimocracy::enterprises::enterprise_execution::validate_establish_enterprise;
use crimocracy::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::intelligence_system::{
    validate_information_transfer, validate_record_information,
};
use crimocracy::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, InformationTransferDraft,
    KnowledgeHolder, Reliability, Specificity,
};
use crimocracy::legal::jurisdiction_system::validate_set_jurisdiction;
use crimocracy::legal::legal_representation_system::validate_retain_legal_representation;
use crimocracy::legal::patrol_system::validate_establish_patrol_deployment;
use crimocracy::legal::prosecution_system::{
    validate_decline_prosecution_case, validate_open_prosecution_case,
};
use crimocracy::legal::{
    Admissibility, ArrestDraft, DayMinute, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, InvestigationDraft, InvestigationWorkKind, InvestigationWorkStatus,
    JurisdictionDraft, LegalRepresentationDraft, PatrolDeploymentDraft, PatrolWindow,
    ProsecutionCaseDraft,
};
use crimocracy::operations::operation_system::{validate_authorize_operation, OperationError};
use crimocracy::operations::property_disposition::{
    validate_dispose_property, PropertyDispositionDraft,
};
use crimocracy::operations::{
    OperationAbortCause, OperationAbortPhase, OperationApproach, OperationContingency,
    OperationDraft, OperationKind, OperationObjective, OperationObjectiveOutcome, OperationStatus,
    RoleKind,
};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
    validate_dismiss_opportunity,
};
use crimocracy::opportunities::{OperationOpportunityDraft, OpportunityStatus};
use crimocracy::recruitment::recruitment_system::validate_recruitment_attempt;
use crimocracy::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crimocracy::registry::Registry;
use crimocracy::reports::{ReportKind, ReportRecord};
use crimocracy::social::relationship_system::validate_set_relationship;
use crimocracy::social::{RelationshipDimensions, RelationshipLevel};
use crimocracy::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crimocracy::world::{
    ApprovalPolicy, AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
    CapabilityKind, CharacterDraft, DriveKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    PolicyKind, PolicySetting, Rating, TraitKind,
};
use crimocracy::{
    contacts::contact_system::{validate_establish_contact, InstitutionalContactDraft},
    legal::arrest_system::validate_arrest,
    legal::investigation_system::{validate_add_evidence, validate_open_investigation},
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

const DEFAULT_BATCH_SAMPLES: u64 = 3;
const MAX_BATCH_SAMPLES: u64 = 64;
const DEFAULT_SEED: u64 = 0x1933_0514;
const MIN_SAMPLES_FOR_VARIATION_CONTRACT: u64 = 3;
/// Minutes of slack added to each operation's authored duration to form the terminal-wait guard,
/// staying registry-derived instead of a hard-coded window that could go stale as authors add
/// longer operations. The guard uses the per-operation duration plus this slack, which is sized
/// to cover police arrival variance and decision deferral for the longest authored operation.
const OPERATION_WAIT_SLACK_MINUTES: u32 = 240;
/// Small deterministic margin past the authored cold-case shelf instant so the narrative re-check
/// observes a case the simulation has already shelved, without depending on in-tick scheduling.
const COLD_CASE_RECHECK_SLACK_MINUTES: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HarnessMode {
    Smoke,
    Full,
}

impl HarnessMode {
    fn parse(value: &str) -> Result<Self, HarnessCliError> {
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
struct HarnessOptions {
    mode: HarnessMode,
    samples: u64,
    seed: u64,
    strategy: Option<Strategy>,
    artifact_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
enum HarnessCliError {
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
enum Strategy {
    Rush,
    Press,
    Recon,
}

impl Strategy {
    fn parse(value: &str) -> Result<Option<Self>, HarnessCliError> {
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

    fn label(self) -> &'static str {
        match self {
            Self::Rush => "RUSH",
            Self::Press => "PRESS",
            Self::Recon => "RECON",
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum HarnessContractError {
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
    #[error(
        "unrelated financial variance changed across strategy branches: legitimate {legitimate:?}; enterprise {enterprise:?}"
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
enum ScenarioProfile {
    NightTrap,
    LatePatrol,
    VeteranCrew,
    ThinCrew,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FixtureVariation {
    Clockwork,
    Crowded,
    Quiet,
}

impl FixtureVariation {
    fn from_seed(seed: u64) -> Self {
        match seed % 3 {
            0 => Self::Clockwork,
            1 => Self::Crowded,
            2 => Self::Quiet,
            _ => unreachable!("seed remainder modulo three is bounded"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Clockwork => "CLOCKWORK",
            Self::Crowded => "CROWDED",
            Self::Quiet => "QUIET",
        }
    }

    fn neighborhood_name(self) -> &'static str {
        match self {
            Self::Clockwork => "South Ward",
            Self::Crowded => "Market Row",
            Self::Quiet => "Canal District",
        }
    }

    fn target_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Bellmore Jewelry",
            Self::Crowded => "Calder's Jewelers",
            Self::Quiet => "Vesper Gold",
        }
    }

    fn alternate_target_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Bellmore Service Annex",
            Self::Crowded => "Calder's Receiving House",
            Self::Quiet => "Vesper Gold Annex",
        }
    }

    fn alternate_source_summary(self) -> &'static str {
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

    fn front_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Fulton Social Club",
            Self::Crowded => "Lantern Room",
            Self::Quiet => "Marlowe Club",
        }
    }

    fn resale_name(self) -> &'static str {
        match self {
            Self::Clockwork => "Mercer Pawn & Exchange",
            Self::Crowded => "Redline Exchange",
            Self::Quiet => "Northline Exchange",
        }
    }

    fn opportunity_summary(self) -> &'static str {
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

    fn source_summary(self) -> &'static str {
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

    fn source_reliability(self) -> Reliability {
        match self {
            Self::Clockwork => Reliability::Mixed,
            Self::Crowded => Reliability::GenerallyReliable,
            Self::Quiet => Reliability::GenerallyReliable,
        }
    }

    fn source_specificity(self) -> Specificity {
        match self {
            Self::Clockwork => Specificity::General,
            Self::Crowded => Specificity::Specific,
            Self::Quiet => Specificity::General,
        }
    }

    fn neighborhood_police_presence(self) -> u8 {
        match self {
            Self::Clockwork => 58,
            Self::Crowded => 72,
            Self::Quiet => 42,
        }
    }

    fn neighborhood_economy(self) -> (u8, u8, u8) {
        match self {
            Self::Clockwork => (62, 78, 72),
            Self::Crowded => (74, 91, 84),
            Self::Quiet => (48, 61, 56),
        }
    }

    fn patrol_windows(self, profile: ScenarioProfile) -> [(u16, u16, u8); 2] {
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
    const SENSITIVITY_SET: [Self; 3] = [Self::LatePatrol, Self::VeteranCrew, Self::ThinCrew];

    fn label(self) -> &'static str {
        match self {
            Self::NightTrap => "NIGHT TRAP",
            Self::LatePatrol => "LATE PATROL",
            Self::VeteranCrew => "VETERAN CREW",
            Self::ThinCrew => "THIN CREW",
        }
    }

    fn lieutenant_management(self) -> u8 {
        match self {
            Self::VeteranCrew => 95,
            Self::ThinCrew => 60,
            Self::NightTrap | Self::LatePatrol => 78,
        }
    }

    fn burglar_burglary(self) -> u8 {
        match self {
            Self::VeteranCrew => 96,
            Self::ThinCrew => 62,
            Self::NightTrap | Self::LatePatrol => 82,
        }
    }

    fn burglar_stealth(self) -> u8 {
        match self {
            Self::VeteranCrew => 92,
            Self::ThinCrew => 58,
            Self::NightTrap | Self::LatePatrol => 76,
        }
    }

    fn scout_surveillance(self) -> u8 {
        match self {
            Self::VeteranCrew => 94,
            Self::ThinCrew => 72,
            Self::NightTrap | Self::LatePatrol => 90,
        }
    }

    fn scout_stealth(self) -> u8 {
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
struct ScenarioTimeline {
    initial_burglary_at: SimTime,
    initial_opportunity_valid_until: SimTime,
    second_opportunity_discovery_at: SimTime,
    second_opportunity_valid_until: SimTime,
    rush_second_act_at: SimTime,
    recon_second_act_surveillance_at: SimTime,
}

impl ScenarioTimeline {
    fn for_scenario(registry: &Registry, seed: u64) -> Self {
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

struct Scenario<'registry> {
    registry: &'registry Registry,
    state: AppState,
    player: OrganizationId,
    rival: OrganizationId,
    second_rival: OrganizationId,
    police: OrganizationId,
    neighborhood: crimocracy::core::id::NeighborhoodId,
    target: BusinessId,
    alternate_target: BusinessId,
    front: BusinessId,
    resale_venue: BusinessId,
    liquidation_cash: crimocracy::core::id::FinancialAccountId,
    liquidation_settlement: crimocracy::core::id::FinancialAccountId,
    boss: CharacterId,
    lieutenant: CharacterId,
    burglar: CharacterId,
    scout: CharacterId,
    /// Player-side replacement candidate for act 2: an independent with a pre-existing personal
    /// relationship to the boss, recruitable only through canonical executive recruitment.
    danny_ferro: CharacterId,
    detective: CharacterId,
    opportunity_information: InformationId,
    alternate_opportunity_information: InformationId,
    enterprise: EnterpriseId,
    investigation: Option<crimocracy::core::id::InvestigationId>,
    variation: FixtureVariation,
    timeline: ScenarioTimeline,
}

#[derive(Clone, Debug, Default)]
struct RunMetrics {
    strategy: Option<Strategy>,
    variation: Option<FixtureVariation>,
    burglary: Option<OperationId>,
    outcome: Option<OperationObjectiveOutcome>,
    aborted: bool,
    abort_phase: Option<OperationAbortPhase>,
    abort_cause: Option<OperationAbortCause>,
    police_dispatched: bool,
    police_arrived: bool,
    decision_requests: u32,
    player_police_activity_information: u32,
    planning_information_count: usize,
    planning_information_topics: BTreeSet<InformationTopic>,
    counterintelligence_outcome: Option<OperationObjectiveOutcome>,
    counterintelligence_information: usize,
    followup_case_active: Option<bool>,
    cold_case_confirmed: Option<bool>,
    case_cold_minute: Option<u64>,
    case_open_minute: Option<u64>,
    counterintelligence_scheduled_at: Option<u64>,
    exposure_score: Option<i16>,
    exposure_level: Option<crimocracy::operations::OperationExposureLevel>,
    investigation_created: bool,
    evidence_count: usize,
    investigation_work_scheduled: u32,
    investigation_work_resolved: u32,
    burglary_information_quality: Option<u8>,
    property_acquired_value_cents: Option<i64>,
    property_realized_cash_cents: Option<i64>,
    burglary_terminal_minute: Option<u64>,
    liquidation_minute: Option<u64>,
    legitimate_net_cents: Option<i64>,
    enterprise_net_cents: Option<i64>,
    discovered_surveillance_information: usize,
    player_legal_activity_information: usize,
    player_report_count: usize,
    executive_brief_count: usize,
    autonomous_recruitment_attempts: u32,
    player_personnel_departures: u32,
    defector: Option<crimocracy::core::id::CharacterId>,
    defection_minute: Option<u64>,
    defector_trail_confirmed: Option<bool>,
    // Act-2 (second wind) evidence: the narrative branches either rebuild and recover value on a
    // reopened second score or deliberately let it lapse as the price of standing down.
    second_opportunity: Option<OpportunityId>,
    second_opportunity_discovered: bool,
    second_opportunity_expired: bool,
    replacement: Option<crimocracy::core::id::CharacterId>,
    replacement_recruited: bool,
    second_burglary: Option<OperationId>,
    second_burglary_aborted: bool,
    second_burglary_outcome: Option<OperationObjectiveOutcome>,
    second_burglary_terminal_minute: Option<u64>,
    second_act_recon_information: usize,
    second_act_property_acquired_value_cents: Option<i64>,
    second_act_property_realized_cash_cents: Option<i64>,
}

#[derive(Default)]
struct Aggregate {
    samples: u64,
    fixture_variations: BTreeSet<FixtureVariation>,
    achieved: u64,
    partial: u64,
    failed: u64,
    aborted: u64,
    unresolved: u64,
    police_dispatched: u64,
    police_arrived: u64,
    decisions: u64,
    investigations: u64,
    investigation_work_scheduled: u64,
    investigation_work_resolved: u64,
    exposure_total: i64,
    exposure_samples: u64,
    intelligence_total: u64,
    intelligence_samples: u64,
    property_acquired_total_cents: i128,
    property_realized_total_cents: i128,
    burglary_terminal_minute_total: u128,
    burglary_terminal_samples: u64,
    liquidation_minute_total: u128,
    liquidation_samples: u64,
    standing_contingency_aborts: u64,
    legal_activity_information_sessions: u64,
    police_activity_information_sessions: u64,
    followup_case_active_sessions: u64,
    cold_case_confirmed_sessions: u64,
    player_report_total: u64,
    executive_brief_total: u64,
    autonomous_recruitment_attempts: u64,
    player_personnel_departures: u64,
}

impl Aggregate {
    fn add(&mut self, metrics: &RunMetrics) {
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
    }

    fn percent(&self, value: u64) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            (value as f64 * 100.0) / self.samples as f64
        }
    }

    fn print(&self, label: &str) {
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
        println!(
            "{label:<6}  samples {:>2}  fixtures {:?}  achieved {:>5.1}%  partial {:>5.1}%  failed {:>5.1}%  aborted {:>5.1}%  unresolved {:>2}  standing {:>5.1}%  police {:>5.1}%  cases {:>5.1}%  legal intel {:>5.1}%  police intel {:>5.1}%  case hot {:>5.1}%  case cold {:>5.1}%  case work {}/{}  avg exposure {:>5.1}  avg intel {:>5.1}  avg finish {:>5.0}m  avg property {:>8.0}c -> {:>8.0}c cash @ {:>5.0}m  reports {:>3}  briefs {:>3}  rival attempts {:>3}  departures {:>3}",
            self.samples,
            self.fixture_variations,
            self.percent(self.achieved),
            self.percent(self.partial),
            self.percent(self.failed),
            self.percent(self.aborted),
            self.unresolved,
            self.percent(self.standing_contingency_aborts),
            self.percent(self.police_dispatched),
            self.percent(self.investigations),
            self.percent(self.legal_activity_information_sessions),
            self.percent(self.police_activity_information_sessions),
            self.percent(self.followup_case_active_sessions),
            self.percent(self.cold_case_confirmed_sessions),
            self.investigation_work_scheduled,
            self.investigation_work_resolved,
            avg_exposure,
            avg_intelligence,
            avg_terminal_minute,
            avg_acquired_property,
            avg_realized_property,
            avg_liquidation_minute,
            self.player_report_total,
            self.executive_brief_total,
            self.autonomous_recruitment_attempts,
            self.player_personnel_departures,
        );
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_options(std::env::args().skip(1))? else {
        return Ok(());
    };

    match options.mode {
        HarnessMode::Smoke => run_smoke(options.seed, options.strategy),
        HarnessMode::Full => run_full(options),
    }
}

fn run_smoke(seed: u64, selected_strategy: Option<Strategy>) -> Result<(), Box<dyn Error>> {
    let registry = build_registry();
    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!("mode: smoke | seed {seed:#x}");
    let contract = match selected_strategy {
        Some(strategy) => format!(
            "contract: {} canonical strategy path (legal foundation skipped)",
            strategy.label()
        ),
        None => "contract: all canonical strategy paths plus legal foundation".to_owned(),
    };
    println!("{contract}");

    if selected_strategy.is_none() {
        run_legal_foundation_check(&registry)?;
    } else {
        println!("legal foundation: skipped for focused strategy iteration");
    }
    for strategy in [Strategy::Rush, Strategy::Press, Strategy::Recon] {
        if selected_strategy.is_some_and(|selected| selected != strategy) {
            continue;
        }
        let metrics = play_session(
            &registry,
            strategy,
            ScenarioProfile::NightTrap,
            seed,
            false,
            false,
        )?;
        validate_run_metrics(&metrics, false)?;
        validate_strategy_evidence(ScenarioProfile::NightTrap, &metrics)?;
        println!(
            "[SMOKE] {:<5} terminal {:>4}m; {}; police {}; evidence {}; legal intel {}; police intel {}; follow-up {:?}; case hot {:?}; cold {:?}; intelligence {:?}; recruit attempts {}; departures {}",
            strategy.label(),
            metrics.burglary_terminal_minute.unwrap_or_default(),
            terminal_label(&metrics),
            if metrics.police_arrived { "arrived" } else { "none" },
            metrics.evidence_count,
            metrics.player_legal_activity_information,
            metrics.player_police_activity_information,
            metrics.counterintelligence_outcome,
            metrics.followup_case_active,
            metrics.cold_case_confirmed,
            metrics.burglary_information_quality,
            metrics.autonomous_recruitment_attempts,
            metrics.player_personnel_departures,
        );
    }
    match selected_strategy {
        Some(strategy) => println!(
            "[SMOKE PASS] {} canonical harness contract passed",
            strategy.label()
        ),
        None => println!("[SMOKE PASS] all canonical harness contracts passed"),
    }
    Ok(())
}

fn run_full(options: HarnessOptions) -> Result<(), Box<dyn Error>> {
    let wall_start = Instant::now();
    let HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
        artifact_dir,
    } = options;
    debug_assert_eq!(mode, HarnessMode::Full);
    debug_assert!(strategy.is_none());
    let registry = build_registry();
    let artifact_dir = artifact_dir.unwrap_or_else(|| PathBuf::from("target/harness"));

    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!("===========================\n");
    println!("Mode: controlled/calibration strategy comparison with bounded scenario sensitivity.");
    println!(
        "Evidence boundary: synthetic setup through production paths; policy inputs are player-visible, while [DEV AUDIT] is diagnostic only.\n"
    );
    println!(
        "Observation windows: narrative sessions run for two simulated days; matched batches run for one day to keep sensitivity evidence bounded.\n"
    );
    println!("Narrative comparison uses seed {seed:#x}.\n");

    println!("--- CONTROLLED SESSION: RUSH ---");
    let rush = play_session(
        &registry,
        Strategy::Rush,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;
    println!("\n--- CONTROLLED SESSION: PRESS ---");
    let press = play_session(
        &registry,
        Strategy::Press,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;
    println!("\n--- CONTROLLED SESSION: RECON ---");
    let recon = play_session(
        &registry,
        Strategy::Recon,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;

    println!("\n--- SAME-SCENARIO READOUT ---");
    validate_run_metrics(&rush, true)?;
    validate_run_metrics(&press, true)?;
    validate_run_metrics(&recon, true)?;
    validate_night_trap_evidence(&rush)?;
    validate_night_trap_evidence(&press)?;
    validate_night_trap_evidence(&recon)?;
    validate_press_consequence_arc(&press)?;
    validate_defector_trail_evidence(&rush)?;
    validate_defector_trail_evidence(&press)?;
    validate_defector_trail_evidence(&recon)?;
    validate_second_act_evidence(&rush)?;
    validate_second_act_evidence(&press)?;
    validate_second_act_evidence(&recon)?;
    print_metrics(&rush);
    print_metrics(&press);
    print_metrics(&recon);
    print_experience_readout(&rush, &press, &recon);
    validate_branch_financial_isolation(&rush, &press, &recon)?;
    println!(
        "[HARNESS CHECK] Legitimate cashflow stayed identical across branches; delegated enterprise cashflow diverged only by the district heat surcharge from an active investigation (PRESS penalized while hot)."
    );

    println!("\n--- OPPORTUNITY PORTFOLIO PROBE ---");
    run_opportunity_portfolio_probe(&registry, seed)?;

    println!("\n--- ORGANIZATIONAL CAPACITY PROBE ---");
    run_organizational_capacity_probe(&registry, seed)?;

    println!("\n--- LEGAL FOUNDATION CHECK ---");
    run_legal_foundation_check(&registry)?;

    println!("\n--- NIGHT-TRAP BATCH ({samples} seeds per strategy) ---");
    println!("[BATCH] Running matched seeds for NIGHT TRAP...");
    let (rush_aggregate, press_aggregate, recon_aggregate) = run_strategy_batch(
        &registry,
        ScenarioProfile::NightTrap,
        samples,
        seed,
        Some(&artifact_dir),
    )?;
    println!("[BATCH PASS] NIGHT TRAP matched-seed checks passed.");
    rush_aggregate.print("RUSH");
    press_aggregate.print("PRESS");
    recon_aggregate.print("RECON");
    println!(
        "Decisions surfaced: rush {}, press {}, recon {}. Police arrivals: rush {}, press {}, recon {}.",
        rush_aggregate.decisions,
        press_aggregate.decisions,
        recon_aggregate.decisions,
        rush_aggregate.police_arrived,
        press_aggregate.police_arrived,
        recon_aggregate.police_arrived,
    );

    println!("\n--- SCENARIO SENSITIVITY ({samples} seeds per strategy/profile) ---");
    for profile in ScenarioProfile::SENSITIVITY_SET {
        println!("[BATCH] Running matched seeds for {}...", profile.label());
        let (rush, press, recon) =
            run_strategy_batch(&registry, profile, samples, seed, Some(&artifact_dir))?;
        println!("\n[{}]", profile.label());
        rush.print("RUSH");
        press.print("PRESS");
        recon.print("RECON");
        println!(
            "[BATCH PASS] {} matched-seed checks passed.",
            profile.label()
        );
    }

    // Persist per-run seeds and raw metrics beneath aggregate diagnostics.
    // Full mode always writes artifacts; the directory defaults to target/harness.
    println!("\n--- ARTIFACTS ---");
    let narrative_runs = [(&rush, seed), (&press, seed), (&recon, seed)];
    for (metrics, run_seed) in narrative_runs {
        let _ = persist_run_artifact(&artifact_dir, run_seed, ScenarioProfile::NightTrap, metrics);
    }
    // Also capture the batch aggregate summary.
    {
        fs::create_dir_all(&artifact_dir)?;
        let summary = serde_json::json!({
            "mode": "full",
            "seed": format!("{seed:#x}"),
            "samples": samples,
            "elapsed_secs": wall_start.elapsed().as_secs_f64(),
            "note": "per-run JSON files retain per-run seeds and raw metrics beneath derived findings"
        });
        let path = artifact_dir.join(format!("summary-{seed:#x}.json"));
        fs::write(&path, serde_json::to_string_pretty(&summary)?)?;
        println!("[ARTIFACT] wrote {}", path.display());
    }
    println!(
        "\n[HARNESS DONE] full suite in {:.1}s  artifacts: {}",
        wall_start.elapsed().as_secs_f64(),
        artifact_dir.display()
    );

    Ok(())
}

fn parse_options(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Option<HarnessOptions>, HarnessCliError> {
    let mut arguments = arguments.into_iter();
    let mut mode = HarnessMode::Smoke;
    let mut samples = DEFAULT_BATCH_SAMPLES;
    let mut seed = DEFAULT_SEED;
    let mut strategy = None;
    let mut strategy_was_passed = false;
    let mut samples_were_explicit = false;
    let mut artifact_dir: Option<PathBuf> = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                print_usage();
                return Ok(None);
            }
            "--mode" => {
                let value = arguments
                    .next()
                    .ok_or(HarnessCliError::MissingValue { flag: "--mode" })?;
                mode = HarnessMode::parse(&value)?;
            }
            "--samples" => {
                samples_were_explicit = true;
                let value = arguments
                    .next()
                    .ok_or(HarnessCliError::MissingValue { flag: "--samples" })?;
                samples = value
                    .parse::<u64>()
                    .map_err(|_| HarnessCliError::InvalidValue {
                        flag: "--samples",
                        value,
                    })?;
                if !(1..=MAX_BATCH_SAMPLES).contains(&samples) {
                    return Err(HarnessCliError::SampleCountOutOfRange { value: samples });
                }
            }
            "--seed" => {
                let value = arguments
                    .next()
                    .ok_or(HarnessCliError::MissingValue { flag: "--seed" })?;
                let normalized = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .unwrap_or(&value);
                seed = u64::from_str_radix(normalized, 16).map_err(|_| {
                    HarnessCliError::InvalidValue {
                        flag: "--seed",
                        value,
                    }
                })?;
            }
            "--strategy" => {
                let value = arguments
                    .next()
                    .ok_or(HarnessCliError::MissingValue { flag: "--strategy" })?;
                strategy = Strategy::parse(&value)?;
                strategy_was_passed = true;
            }
            "--artifact-dir" => {
                let value = arguments.next().ok_or(HarnessCliError::MissingValue {
                    flag: "--artifact-dir",
                })?;
                artifact_dir = Some(PathBuf::from(value));
            }
            _ => {
                return Err(HarnessCliError::UnsupportedArgument { argument });
            }
        }
    }
    if mode == HarnessMode::Smoke {
        if samples_were_explicit && samples != 1 {
            return Err(HarnessCliError::SmokeSampleCount { value: samples });
        }
        samples = 1;
    } else if strategy_was_passed {
        return Err(HarnessCliError::StrategyOnlyInSmoke);
    }
    Ok(Some(HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
        artifact_dir,
    }))
}

fn print_usage() {
    println!(
        "Usage: cargo run --example gameplay_harness -- [--mode smoke|full] [--strategy all|rush|press|recon] [--samples 1..={MAX_BATCH_SAMPLES}] [--seed HEX] [--artifact-dir DIR]"
    );
    println!("  smoke  Fast canonical-path check for the local gate and iteration (default).");
    println!("         --strategy rush|press|recon focuses one branch; default is all.");
    println!("  full   Narrative session, legal check, matched batch, and sensitivity report.");
    println!("         --artifact-dir writes per-run JSON artifacts (default: target/harness/).");
}

fn validate_run_metrics(
    metrics: &RunMetrics,
    require_financials: bool,
) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    if metrics.burglary.is_none() {
        return Err(HarnessContractError::MissingBurglary { strategy });
    }
    if metrics.burglary_terminal_minute.is_none() {
        return Err(HarnessContractError::MissingTerminalState { strategy });
    }
    if metrics.aborted == metrics.outcome.is_some()
        || metrics.aborted != (metrics.abort_phase.is_some() && metrics.abort_cause.is_some())
        || (!metrics.aborted && (metrics.abort_phase.is_some() || metrics.abort_cause.is_some()))
    {
        return Err(HarnessContractError::InconsistentTerminalState {
            strategy,
            aborted: metrics.aborted,
            outcome: metrics.outcome,
            abort_phase: metrics.abort_phase,
            abort_cause: metrics.abort_cause,
        });
    }
    if require_financials
        && (metrics.legitimate_net_cents.is_none() || metrics.enterprise_net_cents.is_none())
    {
        return Err(HarnessContractError::MissingFinancialObservation { strategy });
    }
    Ok(())
}

fn validate_night_trap_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    let evidence = match strategy {
        Strategy::Rush => {
            if matches!(
                metrics.abort_cause,
                Some(OperationAbortCause::PoliceArrival(_))
            ) && metrics.abort_phase == Some(OperationAbortPhase::InProgress)
            {
                None
            } else {
                Some("pre-entry police arrival triggers the standing abort contingency")
            }
        }
        Strategy::Press => {
            if metrics.decision_requests > 0
                && metrics.investigation_created
                && metrics.player_legal_activity_information > 0
                && metrics.player_police_activity_information > 0
                && metrics.counterintelligence_outcome.is_some()
                && metrics.counterintelligence_information > 0
                && metrics.followup_case_active == Some(true)
            {
                None
            } else {
                Some("police-arrival decision, surfaced field/legal information, and a counter-surveillance follow-up that reads whether the case is still active")
            }
        }
        Strategy::Recon => {
            if metrics.discovered_surveillance_information >= 2
                && metrics.planning_information_count >= 3
                && metrics
                    .planning_information_topics
                    .contains(&InformationTopic::MarketAccess)
                && metrics.outcome.is_some()
            {
                None
            } else {
                Some("surveillance information must carry both patrol and venue-access facts into the burglary plan")
            }
        }
    };
    evidence.map_or(Ok(()), |evidence| {
        Err(HarnessContractError::MissingStrategyEvidence { strategy, evidence })
    })
}

fn validate_strategy_evidence(
    profile: ScenarioProfile,
    metrics: &RunMetrics,
) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    match strategy {
        Strategy::Rush if profile != ScenarioProfile::LatePatrol => {
            validate_night_trap_evidence(metrics)
        }
        Strategy::Press if metrics.police_arrived => validate_night_trap_evidence(metrics),
        Strategy::Recon => validate_night_trap_evidence(metrics),
        Strategy::Rush | Strategy::Press => Ok(()),
    }
}

/// Full-mode Press narrative must complete the whole consequence arc: the player follows up,
/// reads that the case is hot, waits out the authored cold window, and verifies the case was
/// shelved through their own surveillance rather than hidden case access.
fn validate_press_consequence_arc(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    if metrics.followup_case_active == Some(true)
        && metrics.cold_case_confirmed == Some(true)
        && metrics.case_cold_minute.is_some()
        && metrics.case_cold_minute.unwrap_or_default()
            > metrics.burglary_terminal_minute.unwrap_or_default()
    {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the surfaced case must cool through the authored cold window and the player's own re-check must confirm the shelf",
        })
    }
}

/// Full-mode narrative sessions must close the defection loop: whenever an autonomous rival
/// departure actually removed a player member, the player's own surveillance of every known rival
/// must confirm where that member landed. A session without a departure must not fabricate a trail.
fn validate_defector_trail_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    if metrics.player_personnel_departures > 0 {
        if metrics.defector_trail_confirmed == Some(true) {
            Ok(())
        } else {
            Err(HarnessContractError::MissingStrategyEvidence {
                strategy,
                evidence: "after an autonomous rival departure removed a player member, the player's own surveillance of every known rival must confirm where the member landed",
            })
        }
    } else if metrics.defector_trail_confirmed.is_none() {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "a session without an autonomous rival departure must not fabricate a defector trail",
        })
    }
}

/// Full-mode narrative sessions must close the second-wind arc through canonical production paths:
/// every branch discovers the reopened second score at the same minute, then either rebuilds and
/// recovers value from it (RUSH via executive recruitment + morning-lull hit, RECON via fresh
/// recon + patrol-safe window) or deliberately lets it lapse as the price of standing down (PRESS).
fn validate_second_act_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    let evidence = match strategy {
        Strategy::Rush => {
            if metrics.second_opportunity_discovered
                && metrics.replacement_recruited
                && metrics.replacement.is_some()
                && metrics.second_burglary.is_some()
                && metrics.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                && metrics.second_act_recon_information == 0
                && metrics.second_burglary_terminal_minute.is_some()
                && metrics.player_personnel_departures > 0
            {
                None
            } else {
                Some("the RUSH second act must discover the reopened score, rebuild the crew after at least one rival departure through the canonical executive path, and work the second score in the morning lull with the rebuilt crew and no fresh recon")
            }
        }
        Strategy::Recon => {
            if metrics.second_opportunity_discovered
                && metrics.second_burglary.is_some()
                && metrics.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                && metrics.second_act_recon_information > 0
                && metrics.second_burglary_terminal_minute.is_some()
            {
                None
            } else {
                Some("the RECON second act must discover the reopened score, re-run surveillance on the alternate target, and complete the burglary inside a fresh patrol-safe window")
            }
        }
        Strategy::Press => {
            if metrics.second_opportunity_discovered
                && metrics.second_opportunity_expired
                && metrics.second_burglary.is_none()
                && !metrics.replacement_recruited
            {
                None
            } else {
                Some("the PRESS second act must discover the reopened score and deliberately let it lapse while standing down, without recruiting or working the second score")
            }
        }
    };
    evidence.map_or(Ok(()), |evidence| {
        Err(HarnessContractError::MissingStrategyEvidence { strategy, evidence })
    })
}

fn validate_harness_state(registry: &Registry, state: &AppState) -> Result<(), Box<dyn Error>> {
    validate_state(state)?;
    validate_state_against_registry(registry, state)?;
    Ok(())
}

fn run_legal_foundation_check(registry: &Registry) -> Result<(), Box<dyn Error>> {
    let mut state = AppState::new(0x1E6A_1933);

    let sponsor = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let police = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )?;
    let firm = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Vale & Mercer".to_owned(),
            kind: OrganizationKind::LegalServices,
        },
    )?;
    let prosecutor_office = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor District Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )?;

    let handler = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Harbor Legal Liaison".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let defendant = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Harbor Associate".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let counsel = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Elena Vale".to_owned(),
            organization: Some(firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(87))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let prosecutor = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Ada Mercer".to_owned(),
            organization: Some(prosecutor_office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;

    validate_set_relationship(
        &state,
        handler,
        counsel,
        RelationshipDimensions {
            trust: level(68),
            respect: level(72),
            fear: level(0),
            affection: level(12),
            dependence: level(25),
            resentment: level(0),
            debt: level(10),
        },
    )?
    .commit(&mut state);
    let contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: counsel,
        },
    )?
    .commit(&mut state)?;

    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Harbor arrest matter".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(defendant)]),
        },
    )?
    .commit(&mut state)?;
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(defendant),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )?
    .commit(&mut state)?;
    let arrest = validate_arrest(
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([evidence]),
        },
    )?
    .commit(&mut state)?;

    let payer = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::AccountedFunds,
            label: "Harbor legal reserve".to_owned(),
        },
    )?;
    let reserve_source = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::Settlement,
            label: "Harbor legal reserve source".to_owned(),
        },
    )?;
    let provider = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(firm),
            kind: AccountKind::LegitimateOperating,
            label: "Vale & Mercer client receipts".to_owned(),
        },
    )?;
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Fund legal reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve_source,
                    amount: Money::from_cents(-20_000),
                },
                LedgerPosting {
                    account: payer,
                    amount: Money::from_cents(20_000),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut state)?;
    let representation = validate_retain_legal_representation(
        &state,
        LegalRepresentationDraft {
            arrest,
            sponsor,
            contact,
            fee: Money::from_cents(5_000),
            payer_account: payer,
            provider_account: provider,
            authorization: None,
        },
    )?
    .commit(&mut state)?;

    let prosecution_case = validate_open_prosecution_case(
        &state,
        ProsecutionCaseDraft {
            arrest,
            prosecutor_office,
            lead_prosecutor: prosecutor,
            evidence: BTreeSet::from([evidence]),
        },
    )?
    .commit(&mut state)?;

    validate_harness_state(registry, &state)?;
    let representation_record = state
        .legal()
        .get_legal_representation(representation)
        .ok_or("legal representation disappeared from harness state")?;
    let prosecution_record = state
        .legal()
        .get_prosecution_case(prosecution_case)
        .ok_or("prosecution case disappeared from harness state")?;
    let evidence_record = state
        .legal()
        .get_evidence(evidence)
        .ok_or("source evidence disappeared from harness state")?;
    if evidence_record.custodian() != police
        || !prosecution_record.evidence().contains(&evidence)
        || representation_record.fee() != Money::from_cents(5_000)
        || state
            .finance()
            .get_account(provider)
            .is_none_or(|account| account.balance() != Money::from_cents(5_000))
    {
        return Err("legal foundation harness invariants did not produce expected state".into());
    }

    validate_decline_prosecution_case(&state, prosecution_case)?.commit(&mut state)?;
    validate_harness_state(registry, &state)?;
    let resolved_case = state
        .legal()
        .get_prosecution_case(prosecution_case)
        .ok_or("resolved prosecution case disappeared from harness state")?;
    if resolved_case.status() != crimocracy::legal::ProsecutionCaseStatus::Declined
        || resolved_case.resolved_at() != Some(state.now())
        || resolved_case.resolution_information().is_none()
        || resolved_case.resolution_report().is_none()
        || state
            .legal()
            .open_prosecution_case_for(arrest, prosecutor_office)
            .is_some()
    {
        return Err("prosecution decline lifecycle did not produce expected state".into());
    }

    println!(
        "[LEGAL PASS] arrest -> paid counsel -> police custody-preserving referral -> terminal prosecution decline"
    );
    Ok(())
}

fn run_strategy_batch(
    registry: &Registry,
    profile: ScenarioProfile,
    samples: u64,
    seed: u64,
    artifact_dir: Option<&PathBuf>,
) -> Result<(Aggregate, Aggregate, Aggregate), Box<dyn Error>> {
    let mut rush_aggregate = Aggregate::default();
    let mut press_aggregate = Aggregate::default();
    let mut recon_aggregate = Aggregate::default();
    for offset in 0..samples {
        let sample_seed = seed.wrapping_add(offset + 1);
        let rush = play_session(registry, Strategy::Rush, profile, sample_seed, false, true)?;
        let press = play_session(registry, Strategy::Press, profile, sample_seed, false, true)?;
        let recon = play_session(registry, Strategy::Recon, profile, sample_seed, false, true)?;
        validate_run_metrics(&rush, true)?;
        validate_run_metrics(&press, true)?;
        validate_run_metrics(&recon, true)?;
        validate_strategy_evidence(profile, &rush)?;
        validate_strategy_evidence(profile, &press)?;
        validate_strategy_evidence(profile, &recon)?;
        validate_branch_financial_isolation(&rush, &press, &recon)?;
        if let Some(dir) = artifact_dir {
            let _ = persist_run_artifact(dir, sample_seed, profile, &rush);
            let _ = persist_run_artifact(dir, sample_seed, profile, &press);
            let _ = persist_run_artifact(dir, sample_seed, profile, &recon);
        }
        rush_aggregate.add(&rush);
        press_aggregate.add(&press);
        recon_aggregate.add(&recon);
    }
    if samples >= MIN_SAMPLES_FOR_VARIATION_CONTRACT {
        let observed = rush_aggregate.fixture_variations.len();
        if observed < 3 {
            return Err(HarnessContractError::InsufficientFixtureVariation {
                profile,
                observed,
                required: 3,
            }
            .into());
        }
    }
    Ok((rush_aggregate, press_aggregate, recon_aggregate))
}

fn validate_branch_financial_isolation(
    rush: &RunMetrics,
    press: &RunMetrics,
    recon: &RunMetrics,
) -> Result<(), HarnessContractError> {
    let same_legitimate_cashflow = rush.legitimate_net_cents == press.legitimate_net_cents
        && press.legitimate_net_cents == recon.legitimate_net_cents;
    if !same_legitimate_cashflow {
        return Err(HarnessContractError::FinancialBranchMismatch {
            legitimate: [
                rush.legitimate_net_cents,
                press.legitimate_net_cents,
                recon.legitimate_net_cents,
            ],
            enterprise: [
                rush.enterprise_net_cents,
                press.enterprise_net_cents,
                recon.enterprise_net_cents,
            ],
        });
    }
    // Enterprise cashflow is intentionally not isolated: active investigations in the
    // district add a heat surcharge to delegated gambling, so PRESS (which created a
    // case) should earn less than RECON in the same district while the case is hot.
    // The second-day narrative shell (case shelved) then converges again; the bound
    // check here keeps the design intentional rather than silent drift.
    let press_penalized =
        press.enterprise_net_cents.unwrap_or(0) <= recon.enterprise_net_cents.unwrap_or(0);
    let rush_penalized =
        rush.enterprise_net_cents.unwrap_or(0) <= recon.enterprise_net_cents.unwrap_or(0);
    if !press_penalized && press.investigation_created {
        return Err(HarnessContractError::FinancialBranchMismatch {
            legitimate: [
                rush.legitimate_net_cents,
                press.legitimate_net_cents,
                recon.legitimate_net_cents,
            ],
            enterprise: [
                rush.enterprise_net_cents,
                press.enterprise_net_cents,
                recon.enterprise_net_cents,
            ],
        });
    }
    if !rush_penalized && rush.investigation_created {
        return Err(HarnessContractError::FinancialBranchMismatch {
            legitimate: [
                rush.legitimate_net_cents,
                press.legitimate_net_cents,
                recon.legitimate_net_cents,
            ],
            enterprise: [
                rush.enterprise_net_cents,
                press.enterprise_net_cents,
                recon.enterprise_net_cents,
            ],
        });
    }
    Ok(())
}

fn persist_run_artifact(
    dir: &PathBuf,
    seed: u64,
    profile: ScenarioProfile,
    metrics: &RunMetrics,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let strategy_label = metrics
        .strategy
        .map(|s| s.label().to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());
    let filename = format!(
        "{}-{:016x}-{}-{}.json",
        profile.label().to_lowercase().replace(' ', "-"),
        seed,
        strategy_label,
        metrics
            .variation
            .map(|v| v.label().to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    let path = dir.join(filename);
    let payload = serde_json::json!({
        "seed": format!("{seed:#x}"),
        "seed_dec": seed,
        "profile": profile.label(),
        "strategy": metrics.strategy.map(|s| s.label()),
        "variation": metrics.variation.map(|v| v.label()),
        "burglary": metrics.burglary.map(|id| format!("{id:?}")),
        "outcome": metrics.outcome.map(|o| format!("{o:?}")),
        "aborted": metrics.aborted,
        "abort_phase": metrics.abort_phase.map(|p| format!("{p:?}")),
        "abort_cause": metrics.abort_cause.map(|c| format!("{c:?}")),
        "police_dispatched": metrics.police_dispatched,
        "police_arrived": metrics.police_arrived,
        "decision_requests": metrics.decision_requests,
        "exposure_score": metrics.exposure_score,
        "evidence_count": metrics.evidence_count,
        "burglary_terminal_minute": metrics.burglary_terminal_minute,
        "legitimate_net_cents": metrics.legitimate_net_cents,
        "enterprise_net_cents": metrics.enterprise_net_cents,
        "player_report_count": metrics.player_report_count,
        "executive_brief_count": metrics.executive_brief_count,
        "raw": {
            "second_opportunity_discovered": metrics.second_opportunity_discovered,
            "second_burglary": metrics.second_burglary.map(|id| format!("{id:?}")),
            "defector_trail_confirmed": metrics.defector_trail_confirmed,
        }
    });
    fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
    println!("[ARTIFACT] wrote {}", path.display());
    Ok(())
}

fn play_session(
    registry: &Registry,
    strategy: Strategy,
    profile: ScenarioProfile,
    seed: u64,
    narrative: bool,
    continue_for_financial_day: bool,
) -> Result<RunMetrics, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, profile)?;
    let mut metrics = RunMetrics {
        strategy: Some(strategy),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };

    if narrative {
        println!(
            "[FIXTURE] {} authored variation selected by simulation seed.",
            scenario.variation.label(),
        );
        print_starting_player_view(&scenario);
    }

    let opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: scenario.variation.opportunity_summary().to_owned(),
            valid_until: Some(scenario.timeline.initial_opportunity_valid_until),
        },
    )?
    .commit(&mut scenario.state)?;

    if narrative {
        let record = scenario
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("committed opportunity must be queryable");
        println!("\n[OBSERVE] Opportunity: {}", record.summary());
        println!(
            "          Source: {}",
            scenario
                .state
                .intelligence()
                .get_information(scenario.opportunity_information)
                .expect("starting information must exist")
                .summary()
        );
    }

    let mut burglary_intelligence = BTreeSet::from([scenario.opportunity_information]);
    let mut learned_patrol_summary = None;
    if strategy == Strategy::Recon {
        if narrative {
            println!(
                "[DECIDE]  Order surveillance before committing the burglary. The goal is to learn venue access and police rhythm."
            );
        }
        let surveillance = authorize_surveillance(&mut scenario)?;
        run_until_operation_terminal(&mut scenario, surveillance, narrative, &mut metrics)?;
        let resolution = scenario
            .state
            .operations()
            .get_operation(surveillance)
            .expect("surveillance must remain queryable")
            .resolution()
            .expect("completed surveillance must have a resolution");
        metrics.discovered_surveillance_information = resolution.discovered_information().len();
        for information in resolution.discovered_information() {
            let record = scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("surveillance information must persist");
            if narrative {
                println!(
                    "[LEARN]   {:?} / {:?}: {}",
                    record.reliability(),
                    record.specificity(),
                    record.summary()
                );
            }
            if record.topic() == InformationTopic::PoliceActivity {
                learned_patrol_summary = Some(record.summary().to_owned());
            }
            // Every discovered record is already organization-held and target-relevant by the
            // surveillance contract. Carry all of it into the next plan so the harness tests
            // the same information-selection boundary a player would use.
            burglary_intelligence.insert(*information);
        }
    }

    let scheduled_for = match strategy {
        Strategy::Rush | Strategy::Press => scenario.timeline.initial_burglary_at,
        Strategy::Recon => {
            let patrol_summary = learned_patrol_summary.as_deref().ok_or(
                "recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
            )?;
            let duration = scenario
                .registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration();
            let chosen = choose_safe_start_from_patrol_report(
                scenario.state.now(),
                patrol_summary,
                duration,
                SimDuration::from_minutes(60),
                scenario.timeline.initial_opportunity_valid_until,
            )?;
            if narrative {
                println!(
                    "[INTERPRET] Parsed the reported recurring patrol windows and chose minute {} so the authored burglary window stays outside them with a one-hour uncertainty buffer.",
                    chosen.as_minutes()
                );
            }
            chosen
        }
    };
    if narrative && matches!(strategy, Strategy::Rush | Strategy::Press) {
        let clock = format_minute_of_day(scheduled_for.as_minutes());
        match strategy {
            Strategy::Rush => println!(
                "[DECIDE]  Move immediately on the opportunity at {clock}, using only the original street information."
            ),
            Strategy::Press => println!(
                "[DECIDE]  Hit {} at {clock} and press on through a police response unless leadership later orders otherwise.",
                scenario.variation.target_name(),
            ),
            Strategy::Recon => unreachable!("recon narrates its own planning decision"),
        }
    }
    if scenario.state.now() >= scheduled_for {
        return Err(format!(
            "scenario preparation reached minute {} before burglary schedule {}",
            scenario.state.now().as_minutes(),
            scheduled_for.as_minutes()
        )
        .into());
    }
    let target = scenario.target;
    let entry_specialist = scenario.burglar;
    let title = format!("{} burglary", scenario.variation.target_name());
    let burglary = authorize_burglary(
        &mut scenario,
        strategy,
        target,
        &title,
        scheduled_for,
        burglary_intelligence,
        entry_specialist,
    )?;
    validate_convert_opportunity(&scenario.state, opportunity, burglary)?
        .commit(&mut scenario.state)?;
    metrics.burglary = Some(burglary);

    if narrative {
        println!(
            "[COMMIT]  Burglary authorized for minute {} with {:?} approach and {} planning information item(s).",
            scheduled_for.as_minutes(),
            scenario
                .state
                .operations()
                .get_operation(burglary)
                .expect("burglary must exist")
                .approach(),
            scenario
                .state
                .operations()
                .get_operation(burglary)
                .expect("burglary must exist")
                .intelligence()
                .len(),
        );
    }
    metrics.planning_information_count = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must exist")
        .intelligence()
        .len();
    metrics.planning_information_topics = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must exist")
        .intelligence()
        .iter()
        .map(|information| {
            scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("selected planning information must persist")
                .topic()
        })
        .collect();
    if narrative {
        print_planning_inputs(&scenario, burglary);
    }

    run_until_operation_terminal(&mut scenario, burglary, narrative, &mut metrics)?;
    metrics.burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    let burglary_record = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must remain queryable");
    metrics.aborted = burglary_record.status() == OperationStatus::Aborted;
    if metrics.aborted {
        let abort = burglary_record
            .abort_record()
            .expect("aborted burglary must persist its abort provenance");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    }
    if let Some(resolution) = burglary_record.resolution() {
        metrics.outcome = Some(resolution.objective_outcome());
        metrics.exposure_score = Some(resolution.exposure().score());
        metrics.exposure_level = Some(resolution.exposure().level());
        metrics.investigation_created = resolution.exposure().investigation().is_some();
        metrics.evidence_count = resolution.exposure().evidence().len();
        // The case ID itself is developer-audit-only; the player only ever sees the surfaced
        // legal-activity knowledge and their own later surveillance observations.
        scenario.investigation = resolution.exposure().investigation();
        metrics.case_open_minute = scenario
            .investigation
            .map(|_| scenario.state.now().as_minutes());
        metrics.burglary_information_quality =
            Some(resolution.factors().intelligence_quality().value());
        metrics.property_acquired_value_cents = resolution
            .property_proceeds()
            .map(|proceeds| proceeds.estimated_value().cents());

        if narrative {
            let report = scenario
                .state
                .reports()
                .get_report(resolution.after_action_report())
                .expect("after-action report must persist");
            print_report("AFTER-ACTION", report, &scenario);
            println!(
                "[CONSEQUENCE] Exposure {:?} (score {}); police case created: {}; evidence records: {}.",
                resolution.exposure().level(),
                resolution.exposure().score(),
                resolution.exposure().investigation().is_some(),
                resolution.exposure().evidence().len(),
            );
            print_resolution_factors(resolution);
            if let Some(proceeds) = resolution.property_proceeds() {
                println!(
                    "[PROCEEDS] Held property estimated at {}. This is organizational value, not liquid cash.",
                    format_cents(proceeds.estimated_value().cents())
                );
            }
        }
    } else if narrative {
        let abort = burglary_record
            .abort_record()
            .expect("aborted burglary must persist its abort provenance");
        println!(
            "[ABORT] phase {:?}, cause {:?}; objective resolution was not completed.",
            abort.phase(),
            abort.cause(),
        );
        if let Some(artifacts) = abort.artifacts() {
            let report = scenario
                .state
                .reports()
                .get_report(artifacts.report())
                .expect("started abort must persist its after-action report");
            print_report("ABORT REPORT", report, &scenario);
        }
        if strategy == Strategy::Rush && narrative {
            println!(
                "[DECIDE]  The standing abort protected the crew. Walk away from {} tonight; the police rhythm there is not beaten by speed alone.",
                scenario
                    .state
                    .world()
                    .get_neighborhood(scenario.neighborhood)
                    .expect("neighborhood must persist")
                    .name()
            );
        }
    }

    let acquired_property_value = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.property_proceeds())
        .map(|proceeds| proceeds.estimated_value());
    if let Some(estimated_value) = acquired_property_value {
        if narrative {
            println!(
                "[DECIDE]  Move the acquired property through {} rather than leave it as held inventory.",
                scenario
                    .state
                    .world()
                    .get_business(scenario.resale_venue)
                    .expect("resale venue must persist")
                    .name(),
            );
        }
        let disposition = validate_dispose_property(
            scenario.registry,
            &scenario.state,
            PropertyDispositionDraft {
                operation: burglary,
                venue: scenario.resale_venue,
                cash_account: scenario.liquidation_cash,
                settlement_account: scenario.liquidation_settlement,
            },
        )?
        .commit(&mut scenario.state)?;
        metrics.property_realized_cash_cents = Some(disposition.realized_value.cents());
        metrics.liquidation_minute = Some(scenario.state.now().as_minutes());
        if narrative {
            println!(
                "[LIQUIDATE] {} estimated property -> {} realized resale cash.",
                format_cents(estimated_value.cents()),
                format_cents(disposition.realized_value.cents())
            );
        }
    }

    metrics.player_legal_activity_information = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::LegalActivity,
        )
        .filter(|information| information.subject() == EntityRef::Operation(burglary))
        .count();
    if narrative {
        print_player_knowledge_gap(&scenario, burglary);
    }

    // The Press branch exercises a real player follow-up: the organization uses only the
    // surfaced legal-activity report and the crew's field report to authorize counter-surveillance
    // of the precinct itself. The investigation's evidence, lead, and internal ID stay hidden; the
    // follow-up reads only whether the authority is still visibly developing the known case.
    if strategy == Strategy::Press
        && metrics.player_legal_activity_information > 0
        && metrics.player_police_activity_information > 0
    {
        let neighborhood_name = scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("counter-surveillance neighborhood must persist")
            .name()
            .to_owned();
        let police_name = scenario
            .state
            .world()
            .get_organization(scenario.police)
            .expect("police organization must persist")
            .name()
            .to_owned();
        let case_open_minute = metrics
            .case_open_minute
            .expect("press consequence arc requires the surfaced case-open minute");
        let cold_window_for_heat_check =
            scenario.registry.legal().cold_case_window().as_minutes() as u64;
        // Heat check lands well inside the authored cold window: ~1/36th of the window
        // (≈60m for the current 2160m window) bounded to [30,90] so it never drifts outside
        // if authors tune the window.
        let heat_check_delay = (cold_window_for_heat_check / 36).clamp(30, 90);
        let heat_check_at = SimTime::from_minutes(case_open_minute + heat_check_delay);
        if narrative {
            println!(
                "[DECIDE]  A case is open and the crew's field report is back. Hold back on further street work in {neighborhood_name} until leadership knows whether {police_name} is still developing it."
            );
            println!(
                "[DECIDE]  Watch {police_name} itself at {}, about one hour after the case opened, to read whether detectives are still actively working the matter.",
                format_minute_of_day(heat_check_at.as_minutes())
            );
        }
        metrics.counterintelligence_scheduled_at = Some(heat_check_at.as_minutes());
        let counterintelligence_title = format!("{police_name} case-heat check");
        let police = scenario.police;
        let counterintelligence = authorize_surveillance_target(
            &mut scenario,
            EntityRef::Organization(police),
            &counterintelligence_title,
            heat_check_at,
        )?;
        run_until_operation_terminal(&mut scenario, counterintelligence, narrative, &mut metrics)?;
        let operation = scenario
            .state
            .operations()
            .get_operation(counterintelligence)
            .expect("counterintelligence operation must persist");
        if let Some(resolution) = operation.resolution() {
            metrics.counterintelligence_outcome = Some(resolution.objective_outcome());
            metrics.counterintelligence_information = resolution.discovered_information().len();
            metrics.followup_case_active = observe_authority_case_sightline(&scenario, resolution);
        }
        if narrative {
            match metrics.followup_case_active {
                Some(true) => println!(
                    "[VERIFY]  Detectives around {police_name} are still actively developing the case. Keep the district dark."
                ),
                Some(false) => println!(
                    "[VERIFY]  No active case machinery around {police_name}; the matter appears shelved."
                ),
                None => println!(
                    "[VERIFY]  The check did not produce a dependable read on the case's activity."
                ),
            }
        }
        // The narrative session waits out the authored cold-case window and re-checks the
        // precinct through the same player-visible surveillance channel. Batch sessions observe
        // one day and stop while the case is still hot, keeping the matched financial window intact.
        // The re-check lands just past the authored inactivity window plus the initial evidence
        // review the authority runs, so session timing tracks registry content instead of
        // hard-coded minutes drifting from authored content.
        if narrative {
            let case_open_minute = metrics
                .case_open_minute
                .expect("press consequence arc requires the surfaced case-open minute");
            let cold_case_window = scenario.registry.legal().cold_case_window();
            let evidence_review_duration = scenario
                .registry
                .get_investigation_work(InvestigationWorkKind::EvidenceReview)
                .duration();
            let pattern_analysis_duration = scenario
                .registry
                .get_investigation_work(InvestigationWorkKind::PatternAnalysis)
                .duration();
            // The inactivity window may be extended by any work that touches the case. Use the
            // longest authored work duration so the estimate covers EvidenceReview (180m) and
            // PatternAnalysis (360m) without hard-coding either value.
            let longest_work = evidence_review_duration.max(pattern_analysis_duration);
            let mut recheck_at = SimTime::from_minutes(
                case_open_minute
                    + u64::from(cold_case_window.as_minutes())
                    + u64::from(longest_work.as_minutes())
                    + u64::from(COLD_CASE_RECHECK_SLACK_MINUTES),
            );
            // PRESS notices the reopened second score at the same canonical minute every narrative
            // branch does, while it is still standing down. The branch then deliberately schedules
            // nothing on it: the discipline that protects the open case is also an opportunity cost.
            if !metrics.second_opportunity_discovered {
                let discovery_at = scenario.timeline.second_opportunity_discovery_at;
                if scenario.state.now() < discovery_at {
                    run_until(&mut scenario, discovery_at, narrative, &mut metrics)?;
                }
                discover_second_opportunity(&mut scenario, narrative, &mut metrics)?;
                if narrative {
                    println!(
                        "[DECIDE]  The second score is real, but {police_name} is still developing the case. Leadership holds the district dark and takes nothing; the opportunity will be allowed to lapse."
                    );
                }
            }
            run_until(&mut scenario, recheck_at, narrative, &mut metrics)?;
            // The shelf estimate assumes only auto-scheduled work advances the case's
            // last-activity instant. If some later work or evidence event pushed that
            // instant past the estimate, extend once by the longest authored work plus slack so
            // the narrative still observes the deterministic shelf instead of misreading a
            // still-hot case through its own surveillance.
            if metrics.case_cold_minute.is_none() {
                recheck_at = recheck_at
                    + longest_work
                    + SimDuration::from_minutes(COLD_CASE_RECHECK_SLACK_MINUTES);
                run_until(&mut scenario, recheck_at, narrative, &mut metrics)?;
            }
            let recheck_title = format!("{police_name} case re-check");
            let recheck_at = scenario.state.now() + SimDuration::ONE_MINUTE;
            let recheck = authorize_surveillance_target(
                &mut scenario,
                EntityRef::Organization(police),
                &recheck_title,
                recheck_at,
            )?;
            run_until_operation_terminal(&mut scenario, recheck, narrative, &mut metrics)?;
            let recheck_operation = scenario
                .state
                .operations()
                .get_operation(recheck)
                .expect("recheck operation must persist");
            if let Some(resolution) = recheck_operation.resolution() {
                metrics.cold_case_confirmed =
                    observe_authority_case_sightline(&scenario, resolution).map(|active| !active);
            }
            match metrics.cold_case_confirmed {
                Some(true) => println!(
                    "[CONSEQUENCE RESOLVED] The precinct has shelved the case. The standing-down worked: the organization absorbed the exposure, kept the district quiet, and outlasted the investigation without touching hidden case state."
                ),
                Some(false) => println!(
                    "[VERIFY]  The precinct is still developing the case; keep standing down."
                ),
                None => println!(
                    "[VERIFY]  The re-check did not produce a dependable read on the case's activity."
                ),
            }
        }
    }

    if continue_for_financial_day {
        // The authored autonomous-recruitment cadence defines the rivals' campaign day. Sessions
        // observe a whole number of those days so financial windows stay matched while session
        // timing tracks the authored content instead of hard-coded minutes.
        let campaign_day_minutes = u64::from(
            scenario
                .registry
                .recruitment()
                .autonomous_attempt_cadence()
                .as_minutes(),
        );
        let observation_end = if narrative {
            SimTime::from_minutes(campaign_day_minutes * 2)
        } else {
            SimTime::from_minutes(campaign_day_minutes)
        };
        // The rival autonomous-recruitment cadence fires once per campaign day. Narrative sessions
        // pause just past the first day boundary so an accepted defection is observable, let the
        // organization run its own defector watch, then finish the observation window.
        let recruitment_boundary = SimTime::from_minutes(campaign_day_minutes + 1);
        if narrative && observation_end > recruitment_boundary {
            run_until(&mut scenario, recruitment_boundary, narrative, &mut metrics)?;
        } else {
            run_until(&mut scenario, observation_end, narrative, &mut metrics)?;
        }
        // All narrative branches notice the reopened second score at the same canonical minute and
        // then each branch either works it (RUSH rebuild, RECON re-recon) or deliberately lets it
        // lapse as the price of standing down (PRESS, which already discovered it during the
        // cold-case wait above). Batch sessions keep the single-act window for performance.
        if narrative && !metrics.second_opportunity_discovered {
            let discovery_at = scenario.timeline.second_opportunity_discovery_at;
            if scenario.state.now() < discovery_at {
                run_until(&mut scenario, discovery_at, narrative, &mut metrics)?;
            }
            discover_second_opportunity(&mut scenario, narrative, &mut metrics)?;
        }
        if narrative && metrics.defector.is_some() && metrics.defector_trail_confirmed.is_none() {
            run_defector_trail(&mut scenario, narrative, &mut metrics)?;
        }
        if narrative {
            run_second_act(&mut scenario, strategy, narrative, &mut metrics)?;
        }
        if scenario.state.now() < observation_end {
            run_until(&mut scenario, observation_end, narrative, &mut metrics)?;
        }
        let financials = resolve_financial_view(&scenario)?;
        metrics.legitimate_net_cents = Some(financials.legitimate_net_cents);
        metrics.enterprise_net_cents = Some(financials.enterprise_net_cents);
        if narrative {
            print_final_case_audit(&scenario, burglary);
            print_second_act_recap(&scenario, strategy, &metrics);
            print_financial_view(&scenario, financials);
            println!("\n[EXECUTIVE BRIEFS]");
            for report in scenario
                .state
                .reports()
                .reports_for(scenario.player)
                .filter(|report| report.kind() == ReportKind::ExecutiveBrief)
            {
                print_report_condensed("BRIEF", report);
            }
        }
    }

    metrics.player_report_count = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.kind() != ReportKind::ExecutiveBrief)
        .count();
    metrics.executive_brief_count = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.kind() == ReportKind::ExecutiveBrief)
        .count();

    Ok(metrics)
}

fn build_scenario(
    registry: &Registry,
    seed: u64,
    profile: ScenarioProfile,
) -> Result<Scenario<'_>, Box<dyn Error>> {
    let mut state = AppState::new(seed);
    let variation = FixtureVariation::from_seed(seed);
    let timeline = ScenarioTimeline::for_scenario(registry, seed);

    let player = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Marrow Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let rival = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Rosetti Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let second_rival = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "D'Amato Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let police = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Central Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )?;
    let detective = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Harlan Pike".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(90))]),
            traits: BTreeSet::from([TraitKind::Patient]),
            drives: BTreeMap::new(),
        },
    )?;
    designate_player_organization(&mut state, player)?;

    // Slight seed-derived jitter keeps the harness from testing one exact clock every run while
    // preserving deterministic matched-seed comparisons.
    let jitter_rating = ((seed >> 4) % 11) as i16 - 5; // -5..+5
    let jitter_minutes = ((seed % 7) as i16 - 3) * 5; // -15..+15 in 5m steps
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: variation.neighborhood_name().to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(jitter_rating_u8(
                        variation.neighborhood_economy().0,
                        jitter_rating,
                    )),
                    commercial_activity: rating(jitter_rating_u8(
                        variation.neighborhood_economy().1,
                        jitter_rating,
                    )),
                    illicit_demand: rating(jitter_rating_u8(
                        variation.neighborhood_economy().2,
                        jitter_rating,
                    )),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(jitter_rating_u8(
                        variation.neighborhood_police_presence(),
                        jitter_rating,
                    )),
                    political_influence: rating(65),
                    social_cohesion: rating(63),
                    visible_violence_tolerance: rating(24),
                },
            },
        },
    )?;
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(85),
        },
    )?
    .commit(&mut state)?;
    let patrol_windows = variation
        .patrol_windows(profile)
        .into_iter()
        .map(|(start, duration, presence)| {
            let jittered_start = (i32::from(start) + i32::from(jitter_minutes))
                .clamp(0, 1_440 - i32::from(duration)) as u16;
            Ok(PatrolWindow::try_new(
                DayMinute::try_new(jittered_start)?,
                duration,
                rating(jitter_rating_u8(presence, jitter_rating)),
            )?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: patrol_windows,
        },
    )?
    .commit(&mut state)?;

    let boss = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Joseph Marrow".to_owned(),
            organization: Some(player),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::from([
                (CapabilityKind::Management, rating(88)),
                (CapabilityKind::Negotiation, rating(75)),
            ]),
            traits: BTreeSet::from([TraitKind::Patient]),
            drives: BTreeMap::new(),
        },
    )?;
    let lieutenant = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Carlo Venn".to_owned(),
            organization: Some(player),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    rating(profile.lieutenant_management()),
                ),
                (CapabilityKind::Intimidation, rating(73)),
            ]),
            traits: BTreeSet::from([TraitKind::Ambitious, TraitKind::Secretive]),
            drives: BTreeMap::from([(DriveKind::Status, rating(78))]),
        },
    )?;
    let burglar = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Frank Dello".to_owned(),
            organization: Some(player),
            supervisor: Some(lieutenant),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Burglary, rating(profile.burglar_burglary())),
                (CapabilityKind::Stealth, rating(profile.burglar_stealth())),
            ]),
            traits: BTreeSet::from([TraitKind::EasilyFrightened]),
            drives: BTreeMap::from([(DriveKind::Safety, rating(88))]),
        },
    )?;
    let scout = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Mara Vale".to_owned(),
            organization: Some(player),
            supervisor: Some(lieutenant),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Surveillance,
                    rating(profile.scout_surveillance()),
                ),
                (CapabilityKind::Stealth, rating(profile.scout_stealth())),
            ]),
            traits: BTreeSet::from([TraitKind::Cautious]),
            drives: BTreeMap::new(),
        },
    )?;
    let bartender = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Lena Orr".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::SocialAccess, rating(65))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    // Danny Ferro is the act-2 replacement candidate: an independent the organization would need
    // to court through the canonical executive recruitment path after losing a crew member. His
    // Gambling-independent career means Burglary 70 / Stealth 58, and he already carries a
    // pre-existing personal relationship to the boss that makes the pitch deterministic without
    // any RNG or hidden-state reads.
    let danny_ferro = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Danny Ferro".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Burglary, rating(70)),
                (CapabilityKind::Stealth, rating(58)),
            ]),
            traits: BTreeSet::from([TraitKind::Greedy]),
            drives: BTreeMap::from([(DriveKind::Money, rating(80))]),
        },
    )?;
    let rival_recruiter = insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Maria Rosetti".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(60))]),
            traits: BTreeSet::from([TraitKind::Cautious]),
            drives: BTreeMap::new(),
        },
    )?;
    insert_character(
        registry,
        &mut state,
        CharacterDraft {
            name: "Victor D'Amato".to_owned(),
            organization: Some(second_rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::from([TraitKind::Proud]),
            drives: BTreeMap::new(),
        },
    )?;

    validate_set_relationship(
        &state,
        burglar,
        rival_recruiter,
        RelationshipDimensions {
            trust: level(10),
            respect: level(15),
            fear: level(20),
            affection: level(5),
            dependence: level(0),
            resentment: level(8),
            debt: level(0),
        },
    )?
    .commit(&mut state);

    validate_set_relationship(
        &state,
        burglar,
        lieutenant,
        RelationshipDimensions {
            trust: level(95),
            respect: level(90),
            fear: level(5),
            affection: level(85),
            dependence: level(90),
            resentment: level(5),
            debt: level(20),
        },
    )?
    .commit(&mut state);

    // Danny's pitch leverages a long-standing personal debt to Marrow, so the relationship edges
    // run from the candidate to the recruiter and the executive recruitment path stays canonical.
    // The margin calculation reads only this authored relationship plus the registry definitions.
    validate_set_relationship(
        &state,
        danny_ferro,
        boss,
        RelationshipDimensions {
            trust: level(70),
            respect: level(60),
            fear: level(10),
            affection: level(60),
            dependence: level(20),
            resentment: level(0),
            debt: level(40),
        },
    )?
    .commit(&mut state);

    validate_assign_mandate(
        registry,
        &state,
        MandateDraft {
            organization: rival,
            manager: rival_recruiter,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
            )]),
            budget: None,
        },
    )?
    .commit(&mut state)?;

    let target = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.target_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )?;
    let alternate_target = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.alternate_target_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )?;
    let front = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.front_name().to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(player),
        },
    )?;
    let resale_venue = insert_business(
        registry,
        &mut state,
        BusinessDraft {
            name: variation.resale_name().to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::Warehousing,
                BusinessFunction::ResaleMarket,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(player),
        },
    )?;

    let business_operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(front),
            kind: AccountKind::LegitimateOperating,
            label: "Fulton legitimate operating".to_owned(),
        },
    )?;
    let business_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(front),
            kind: AccountKind::Settlement,
            label: "Fulton legitimate settlement".to_owned(),
        },
    )?;
    validate_establish_business_economy(
        registry,
        &state,
        BusinessEconomyDraft {
            business: front,
            operating_account: business_operating,
            settlement_account: business_settlement,
        },
    )?
    .commit(&mut state)?;

    let cash_kind = if seed.is_multiple_of(2) {
        AccountKind::StreetCash
    } else {
        AccountKind::ConcealedCash
    };
    let enterprise_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: cash_kind,
            label: format!("{} cash ({:?})", variation.neighborhood_name(), cash_kind),
        },
    )?;
    let enterprise_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
            label: format!("{} gambling settlement", variation.neighborhood_name()),
        },
    )?;
    let liquidation_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
            label: "Mercer resale cash".to_owned(),
        },
    )?;
    let liquidation_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::Settlement,
            label: "Mercer resale settlement".to_owned(),
        },
    )?;
    let mandate = validate_assign_mandate(
        registry,
        &state,
        MandateDraft {
            organization: player,
            manager: lieutenant,
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(neighborhood),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )?
    .commit(&mut state)?;
    let enterprise = validate_establish_enterprise(
        registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: player,
            authority: MandateAuthority {
                mandate,
                manager: lieutenant,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Business(front),
            supporting_businesses: BTreeSet::new(),
            cash_account: enterprise_cash,
            settlement_account: enterprise_settlement,
        },
    )?
    .commit(&mut state)?;

    let opportunity_information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(player),
            source_kind: InformationSourceKind::StreetRumor,
            topic: InformationTopic::TargetSecurity,
            source_entity: Some(EntityRef::Character(bartender)),
            subject: EntityRef::Business(target),
            observed_at: state.now(),
            reliability: variation.source_reliability(),
            specificity: variation.source_specificity(),
            summary: variation.source_summary().to_owned(),
        },
    )?
    .commit(&mut state)
    .expect("opportunity source information fixture should commit");
    let alternate_opportunity_information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(player),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: Some(EntityRef::Character(bartender)),
            subject: EntityRef::Business(alternate_target),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: variation.alternate_source_summary().to_owned(),
        },
    )?
    .commit(&mut state)
    .expect("alternate opportunity source information fixture should commit");

    let scenario = Scenario {
        registry,
        state,
        player,
        rival,
        second_rival,
        police,
        neighborhood,
        target,
        alternate_target,
        front,
        resale_venue,
        liquidation_cash,
        liquidation_settlement,
        boss,
        lieutenant,
        burglar,
        scout,
        danny_ferro,
        detective,
        opportunity_information,
        alternate_opportunity_information,
        enterprise,
        investigation: None,
        variation,
        timeline,
    };
    validate_harness_state(scenario.registry, &scenario.state)?;
    Ok(scenario)
}

fn authorize_surveillance(scenario: &mut Scenario) -> Result<OperationId, Box<dyn Error>> {
    let title = format!("{} surveillance", scenario.variation.target_name());
    authorize_surveillance_target(
        scenario,
        EntityRef::Business(scenario.target),
        &title,
        scenario.state.now() + SimDuration::ONE_MINUTE,
    )
}

fn authorize_surveillance_target(
    scenario: &mut Scenario,
    target: EntityRef,
    title: &str,
    scheduled_for: SimTime,
) -> Result<OperationId, Box<dyn Error>> {
    Ok(validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: title.to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: scenario.player,
            leader: scenario.scout,
            objective: OperationObjective::GatherInformation { target },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, scenario.scout)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?)
}

/// Describes the surveillance plan level visible to the player from a discovered police-org
/// observation: active-case heat versus a shelved case. Returns `None` when no legal-activity
/// observation about the authority was produced.
fn observe_authority_case_sightline(
    scenario: &Scenario,
    resolution: &crimocracy::operations::OperationResolutionRecord,
) -> Option<bool> {
    resolution
        .discovered_information()
        .iter()
        .find_map(|information| {
            let record = scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("discovered surveillance information must persist");
            if record.topic() != InformationTopic::LegalActivity {
                return None;
            }
            if record.summary().contains("actively developing the case") {
                Some(true)
            } else if record.summary().contains("shelved") {
                Some(false)
            } else {
                None
            }
        })
}

fn authorize_burglary(
    scenario: &mut Scenario,
    strategy: Strategy,
    target: BusinessId,
    title: &str,
    scheduled_for: SimTime,
    intelligence: BTreeSet<InformationId>,
    entry_specialist: CharacterId,
) -> Result<OperationId, Box<dyn Error>> {
    let contingencies = match strategy {
        Strategy::Rush | Strategy::Recon => vec![
            OperationContingency::AbortOnPoliceArrivalBeforeEntry,
            OperationContingency::RequestDecisionOnUnexpectedCondition,
        ],
        Strategy::Press => vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
    };
    Ok(validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: title.to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.lieutenant),
                (RoleKind::EntrySpecialist, entry_specialist),
            ]),
            intelligence,
            constraints: Vec::new(),
            contingencies,
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?)
}

/// The narrative act-2 opening: at the canonical discovery minute every narrative branch sees the
/// alternate target's second score as a fresh player-visible opportunity, committed through the
/// canonical discovery path so conversion, expiry, and their reports all follow production rules.
fn discover_second_opportunity(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<OpportunityId, Box<dyn Error>> {
    let discovered_at = scenario.state.now();
    debug_assert!(
        discovered_at >= scenario.timeline.second_opportunity_discovery_at,
        "second opportunity discovery must not be authored earlier than its scenario timeline"
    );
    let valid_until = scenario.timeline.second_opportunity_valid_until;
    let opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.alternate_target)]),
            source_information: BTreeSet::from([scenario.alternate_opportunity_information]),
            summary: format!(
                "{} is moving high-value stock again; the second score on {} is available until the window closes.",
                scenario.variation.alternate_target_name(),
                scenario
                    .state
                    .world()
                    .get_neighborhood(scenario.neighborhood)
                    .expect("neighborhood must persist")
                    .name(),
            ),
            valid_until: Some(valid_until),
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.second_opportunity = Some(opportunity);
    metrics.second_opportunity_discovered = true;
    if narrative {
        let record = scenario
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("committed second opportunity must be queryable");
        println!("[OBSERVE] {}", stamp(discovered_at.as_minutes()));
        println!("          Opportunity: {}", record.summary());
        println!(
            "          Source: {}",
            scenario
                .state
                .intelligence()
                .get_information(scenario.alternate_opportunity_information)
                .expect("alternate source information must persist")
                .summary()
        );
        println!(
            "          The second score expires at {}.",
            format_minute_of_day(valid_until.as_minutes())
        );
    }
    Ok(opportunity)
}

/// The RUSH rebuild beat: after an autonomous rival departure removed the entry specialist, the
/// player works the canonical executive recruitment path to court the independent candidate. The
/// candidate relationship is authored so acceptance is deterministic and identical across seeds;
/// the recruitment decision itself never reads hidden or audit state.
fn recruit_replacement(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<CharacterId, Box<dyn Error>> {
    let candidate = scenario.danny_ferro;
    let recruiter = scenario.boss;
    let organization = scenario.player;
    let attempt = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            target_organization: organization,
            recruiter,
            candidate,
            approach: RecruitmentApproach::FinancialOpportunity,
        },
    )?
    .commit(&mut scenario.state)?;
    let attempt_record = scenario
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("committed executive recruitment must be queryable");
    if attempt_record.outcome() != crimocracy::recruitment::RecruitmentOutcome::Accepted {
        let candidate_name = scenario
            .state
            .world()
            .get_character(candidate)
            .expect("replacement candidate must persist")
            .name()
            .to_owned();
        return Err(format!(
            "replacement recruitment of {candidate_name} was {:?}; the rebuilt-crew contract requires acceptance",
            attempt_record.outcome()
        )
        .into());
    }
    let record = scenario
        .state
        .world()
        .get_character(candidate)
        .expect("recruited replacement must persist");
    if record.organization() != Some(organization) {
        return Err(
            "replacement recruitment committed without a player-organization membership".into(),
        );
    }
    metrics.replacement = Some(candidate);
    metrics.replacement_recruited = true;
    if narrative {
        let candidate_name = record.name();
        let recruiter_name = scenario
            .state
            .world()
            .get_character(recruiter)
            .expect("recruiter must persist")
            .name();
        let organization_name = scenario
            .state
            .world()
            .get_organization(organization)
            .expect("player organization must persist")
            .name();
        println!(
            "[DECIDE]  Leadership personally recruited {candidate_name}: Marrow made the {:?} pitch and {candidate_name} accepted, joining {organization_name} as the replacement entry specialist.",
            attempt_record.approach()
        );
        println!(
            "[NARRATION] {recruiter_name}'s pitch was backed by an existing relationship and the candidate's greed drive; margin {}, outcome {:?}. No hidden or audit state influenced the decision.",
            attempt_record.margin(),
            attempt_record.outcome()
        );
    }
    Ok(candidate)
}

/// Executes the narrative act-2 beat per branch. RUSH rebuilds the crew and works the second score
/// in the morning lull; RECON re-invests in planning and works it inside a fresh patrol-safe
/// window; PRESS deliberately takes nothing and lets the discovered opportunity lapse.
fn run_second_act(
    scenario: &mut Scenario,
    strategy: Strategy,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(opportunity) = metrics.second_opportunity else {
        return Err("act 2 cannot run before the second opportunity is discovered".into());
    };
    let alternate_target = scenario.alternate_target;
    let neighborhood_name = scenario
        .state
        .world()
        .get_neighborhood(scenario.neighborhood)
        .expect("neighborhood must persist")
        .name()
        .to_owned();

    match strategy {
        Strategy::Rush => {
            let replacement = recruit_replacement(scenario, narrative, metrics)?;
            let scheduled_for = scenario.timeline.rush_second_act_at;
            let title = format!(
                "{} second-score burglary",
                scenario.variation.alternate_target_name()
            );
            if narrative {
                println!(
                    "[DECIDE]  Rebuild is in hand. Work the second score on {} during the morning lull at {}, with the rebuilt crew and the original street observation only — no fresh recon.",
                    scenario.variation.alternate_target_name(),
                    format_minute_of_day(scheduled_for.as_minutes()),
                );
            }
            let intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
            let burglary = authorize_burglary(
                scenario,
                Strategy::Rush,
                alternate_target,
                &title,
                scheduled_for,
                intelligence,
                replacement,
            )?;
            validate_convert_opportunity(&scenario.state, opportunity, burglary)?
                .commit(&mut scenario.state)?;
            metrics.second_burglary = Some(burglary);
            run_until_operation_terminal(scenario, burglary, narrative, metrics)?;
            record_second_act_burglary_terminal(scenario, burglary, metrics);
            liquidate_second_act_property(scenario, burglary, narrative, metrics)?;
        }
        Strategy::Recon => {
            let title = format!(
                "{} second-score surveillance",
                scenario.variation.alternate_target_name()
            );
            if narrative {
                println!(
                    "[DECIDE]  Re-invest in planning: run fresh surveillance on {} before committing the second score, and pick the protected window from the new report.",
                    scenario.variation.alternate_target_name()
                );
            }
            let recon = authorize_surveillance_target(
                scenario,
                EntityRef::Business(alternate_target),
                &title,
                scenario.timeline.recon_second_act_surveillance_at,
            )?;
            run_until_operation_terminal(scenario, recon, narrative, metrics)?;
            let resolution = scenario
                .state
                .operations()
                .get_operation(recon)
                .expect("second-score surveillance must persist")
                .resolution()
                .expect("completed second-score surveillance must have a resolution");
            metrics.second_act_recon_information = resolution.discovered_information().len();
            let mut burglary_intelligence =
                BTreeSet::from([scenario.alternate_opportunity_information]);
            let mut learned_patrol_summary = None;
            for information in resolution.discovered_information() {
                let record = scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("second-score surveillance information must persist");
                if narrative {
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
                if record.topic() == InformationTopic::PoliceActivity {
                    learned_patrol_summary = Some(record.summary().to_owned());
                }
                burglary_intelligence.insert(*information);
            }
            let patrol_summary = learned_patrol_summary.as_deref().ok_or(
                "second-score recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
            )?;
            let duration = scenario
                .registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration();
            let scheduled_for = choose_safe_start_from_patrol_report(
                scenario.state.now(),
                patrol_summary,
                duration,
                SimDuration::from_minutes(60),
                scenario.timeline.second_opportunity_valid_until,
            )?;
            if narrative {
                println!(
                    "[INTERPRET] Parsed the reported recurring patrol windows and chose {} so the second-score burglary stays outside them with a one-hour uncertainty buffer.",
                    format_minute_of_day(scheduled_for.as_minutes())
                );
            }
            let title = format!(
                "{} second-score burglary",
                scenario.variation.alternate_target_name()
            );
            let burglary = authorize_burglary(
                scenario,
                Strategy::Recon,
                alternate_target,
                &title,
                scheduled_for,
                burglary_intelligence,
                scenario.burglar,
            )?;
            validate_convert_opportunity(&scenario.state, opportunity, burglary)?
                .commit(&mut scenario.state)?;
            metrics.second_burglary = Some(burglary);
            run_until_operation_terminal(scenario, burglary, narrative, metrics)?;
            record_second_act_burglary_terminal(scenario, burglary, metrics);
            liquidate_second_act_property(scenario, burglary, narrative, metrics)?;
        }
        Strategy::Press => {
            // PRESS already discovered the second score during the cold-case wait and deliberately
            // scheduled nothing on it. The lapse is a standing-down cost, narrated when the
            // opportunity expired and confirmed here from the public lifecycle record.
            if metrics.second_opportunity_expired {
                if narrative {
                    println!(
                        "[DECIDE]  Standing down has a price: the second score on {} lapsed without action while the case stayed protected. The discipline that outlasted the investigation also gave up real value.",
                        scenario.variation.alternate_target_name()
                    );
                }
            } else if narrative {
                println!(
                    "[DECIDE]  The second score is still on the table, but {neighborhood_name} stays dark until leadership confirms the case is shelved."
                );
            }
        }
    }
    Ok(())
}

fn record_second_act_burglary_terminal(
    scenario: &Scenario,
    burglary: OperationId,
    metrics: &mut RunMetrics,
) {
    metrics.second_burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    let record = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("second-score burglary must remain queryable");
    metrics.second_burglary_aborted = record.status() == OperationStatus::Aborted;
    if let Some(resolution) = record.resolution() {
        metrics.second_burglary_outcome = Some(resolution.objective_outcome());
        metrics.second_act_property_acquired_value_cents = resolution
            .property_proceeds()
            .map(|proceeds| proceeds.estimated_value().cents());
    }
}

fn liquidate_second_act_property(
    scenario: &mut Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(estimated_value) = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.property_proceeds())
        .map(|proceeds| proceeds.estimated_value())
    else {
        return Ok(());
    };
    let venue_name = scenario
        .state
        .world()
        .get_business(scenario.resale_venue)
        .expect("resale venue must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[DECIDE]  Move the second-score property through {venue_name} rather than leave it as held inventory."
        );
    }
    let disposition = validate_dispose_property(
        scenario.registry,
        &scenario.state,
        PropertyDispositionDraft {
            operation: burglary,
            venue: scenario.resale_venue,
            cash_account: scenario.liquidation_cash,
            settlement_account: scenario.liquidation_settlement,
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.second_act_property_realized_cash_cents = Some(disposition.realized_value.cents());
    if narrative {
        println!(
            "[LIQUIDATE] {} estimated property -> {} realized resale cash.",
            format_cents(estimated_value.cents()),
            format_cents(disposition.realized_value.cents())
        );
    }
    Ok(())
}

fn print_second_act_recap(scenario: &Scenario, strategy: Strategy, metrics: &RunMetrics) {
    let target = scenario.variation.alternate_target_name();
    match strategy {
        Strategy::Rush | Strategy::Recon => {
            let outcome = metrics
                .second_burglary_outcome
                .map(|outcome| format!("{outcome:?}"))
                .unwrap_or_else(|| "no resolution".to_owned());
            let realized = optional_dollars(metrics.second_act_property_realized_cash_cents);
            println!(
                "\n[ACT 2] {target} second score: {} at minute {}, liquidating {}.",
                outcome,
                metrics
                    .second_burglary_terminal_minute
                    .map(|minute| minute.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                realized
            );
            if strategy == Strategy::Rush {
                println!(
                    "[ACT 2] Rebuild evidence: replacement recruited through executive recruitment; no fresh recon was used; the rebuilt crew worked the morning lull."
                );
            } else {
                println!(
                    "[ACT 2] Re-plan evidence: fresh surveillance produced {} information item(s) and the burglary used a patrol-safe window.",
                    metrics.second_act_recon_information
                );
            }
        }
        Strategy::Press => {
            let lapsed_at = metrics
                .second_opportunity
                .and_then(|opportunity| scenario.state.opportunities().get_opportunity(opportunity))
                .and_then(|record| record.resolution())
                .map(|resolution| resolution.at().as_minutes().to_string())
                .unwrap_or_else(|| "-".to_owned());
            println!(
                "\n[ACT 2] {target} second score deliberately lapsed at minute {lapsed_at} while the case stayed hot; the standing-down cost the organization the value it refused to risk."
            );
        }
    }
}

fn run_opportunity_portfolio_probe(registry: &Registry, seed: u64) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, ScenarioProfile::NightTrap)?;
    let valid_until = Some(SimTime::from_minutes(180));
    let primary_opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: scenario.variation.opportunity_summary().to_owned(),
            valid_until,
        },
    )?
    .commit(&mut scenario.state)?;
    let alternate_opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.alternate_target)]),
            source_information: BTreeSet::from([scenario.alternate_opportunity_information]),
            summary: format!(
                "{} has directly observed high-value stock available after midnight.",
                scenario.variation.alternate_target_name()
            ),
            valid_until,
        },
    )?
    .commit(&mut scenario.state)?;
    println!(
        "[PORTFOLIO] Two burglary opportunities are open until minute 180: {} (street rumor) and {} (direct, precise observation).",
        scenario.variation.target_name(),
        scenario.variation.alternate_target_name(),
    );

    // This is an explicit player-visible prioritization rule: commit the opportunity with the
    // strongest available source instead of treating every open card as equally actionable.
    let target = scenario.alternate_target;
    let title = format!("{} burglary", scenario.variation.alternate_target_name());
    let intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
    let entry_specialist = scenario.burglar;
    let selected_operation = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        target,
        &title,
        SimTime::from_minutes(130),
        intelligence,
        entry_specialist,
    )?;
    validate_convert_opportunity(&scenario.state, alternate_opportunity, selected_operation)?
        .commit(&mut scenario.state)?;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(selected_operation),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, selected_operation, false, &mut metrics)?;
    metrics.burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    run_until(
        &mut scenario,
        SimTime::from_minutes(181),
        false,
        &mut metrics,
    )?;
    let selected_operation_record = scenario
        .state
        .operations()
        .get_operation(selected_operation)
        .expect("selected portfolio operation must persist");
    metrics.aborted = selected_operation_record.status() == OperationStatus::Aborted;
    if metrics.aborted {
        let abort = selected_operation_record
            .abort_record()
            .expect("aborted portfolio operation must preserve its cause");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    } else {
        metrics.outcome = selected_operation_record
            .resolution()
            .map(|resolution| resolution.objective_outcome());
    }
    validate_harness_state(scenario.registry, &scenario.state)?;
    let selected = scenario
        .state
        .opportunities()
        .get_opportunity(alternate_opportunity)
        .expect("selected opportunity must persist");
    let deferred = scenario
        .state
        .opportunities()
        .get_opportunity(primary_opportunity)
        .expect("deferred opportunity must persist");
    if selected.status() != OpportunityStatus::Converted
        || selected
            .resolution()
            .and_then(|resolution| resolution.operation())
            != Some(selected_operation)
        || deferred.status() != OpportunityStatus::Expired
        || deferred
            .resolution()
            .and_then(|resolution| resolution.report())
            .is_none()
    {
        return Err(
            "portfolio probe did not preserve selected and deferred opportunity lifecycles".into(),
        );
    }
    if let Some(report_id) = deferred
        .resolution()
        .and_then(|resolution| resolution.report())
    {
        let report = scenario
            .state
            .reports()
            .get_report(report_id)
            .expect("expired opportunity report must persist");
        print_report("PORTFOLIO EXPIRY REPORT", report, &scenario);
    }
    // Prove the Dismissed lifecycle is distinct from Expiry: dismiss a fresh opportunity
    // through its canonical path and verify the lifecycle report, proving the harness is not
    // stale on the three non-converted states.
    let dismissable = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: "Dismissable decoy opportunity for lifecycle probe.".to_owned(),
            valid_until: Some(SimTime::from_minutes(500)),
        },
    )?
    .commit(&mut scenario.state)?;
    validate_dismiss_opportunity(&scenario.state, dismissable)?.commit(&mut scenario.state)?;
    let dismissed = scenario
        .state
        .opportunities()
        .get_opportunity(dismissable)
        .expect("dismissed opportunity must persist");
    if dismissed.status() != OpportunityStatus::Dismissed || dismissed.resolution().is_none() {
        return Err("dismiss lifecycle did not produce expected Dismissed state".into());
    }
    println!(
        "[PORTFOLIO] Selected {} from player-visible source quality, converted it into {}, left the weaker opportunity to expire, and dismissed a decoy through the canonical lifecycle.",
        scenario.variation.alternate_target_name(),
        terminal_label(&metrics),
    );
    Ok(())
}

/// Proves that characters are a scarce organizational resource rather than infinitely reusable
/// stat bundles. The probe attempts to commit one specialist to overlapping jobs, observes the
/// typed rejection without mutating state, then retries after the first job reaches a terminal
/// state and confirms that the specialist is available again.
fn run_organizational_capacity_probe(registry: &Registry, seed: u64) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, ScenarioProfile::NightTrap)?;
    let first_start = scenario.timeline.initial_burglary_at;
    let target = scenario.target;
    let opportunity_information = scenario.opportunity_information;
    let burglar = scenario.burglar;
    let first = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        target,
        "capacity probe first burglary",
        first_start,
        BTreeSet::from([opportunity_information]),
        burglar,
    )?;
    let overlapping_start = first_start + SimDuration::from_minutes(1);
    let overlapping = validate_authorize_operation(
        registry,
        &scenario.state,
        OperationDraft {
            title: "capacity probe overlapping burglary".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: scenario.player,
            leader: scenario.boss,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(scenario.alternate_target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.boss),
                (RoleKind::EntrySpecialist, burglar),
            ]),
            intelligence: BTreeSet::from([scenario.alternate_opportunity_information]),
            constraints: Vec::new(),
            contingencies: vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
            scheduled_for: overlapping_start,
        },
    )
    .expect_err("overlapping specialist assignments must be rejected");
    let overlapping_debug = format!("{overlapping:?}");
    let expected_rejection = matches!(
        overlapping,
        OperationError::ParticipantBusy {
            character,
            operation,
        } if character == burglar && operation == first
    );
    if !expected_rejection {
        return Err(format!(
            "capacity probe returned the wrong rejection for overlapping specialist: observed {overlapping_debug}, expected ParticipantBusy {{ character: {burglar:?}, operation: {first:?} }}"
        )
        .into());
    }
    validate_harness_state(registry, &scenario.state)?;
    println!(
        "[CAPACITY] {} was reserved for the first burglary; the overlapping second plan was rejected as {:?} without changing authoritative state.",
        scenario
            .state
            .world()
            .get_character(scenario.burglar)
            .expect("capacity-probe specialist must persist")
            .name(),
        overlapping_debug,
    );

    let mut first_metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(first),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, first, false, &mut first_metrics)?;
    capture_terminal_status(&scenario, first, &mut first_metrics);
    let released_start = scenario.state.now() + SimDuration::ONE_MINUTE;
    let alternate_target = scenario.alternate_target;
    let alternate_opportunity_information = scenario.alternate_opportunity_information;
    let second = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        alternate_target,
        "capacity probe released burglary",
        released_start,
        BTreeSet::from([alternate_opportunity_information]),
        burglar,
    )?;
    let mut second_metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(second),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, second, false, &mut second_metrics)?;
    capture_terminal_status(&scenario, second, &mut second_metrics);
    validate_harness_state(registry, &scenario.state)?;
    println!(
        "[CAPACITY] After the first burglary became {}, {} was released and the second plan authorized at minute {} (terminal {}).",
        terminal_label(&first_metrics),
        scenario
            .state
            .world()
            .get_character(scenario.burglar)
            .expect("capacity-probe specialist must persist")
            .name(),
        released_start.as_minutes(),
        terminal_label(&second_metrics),
    );
    // Prove delegation lifecycle is not stale: revise the player's mandate to add a standing
    // order, verify the version advances, then ensure state remains valid. This exercises
    // the canonical revise path that earlier harness iterations never touched.
    let player_mandate = scenario
        .state
        .delegation()
        .active_for_manager(scenario.lieutenant)
        .map(|record| record.id())
        .expect("player mandate must still be active after capacity probe");
    let mandate_record = scenario
        .state
        .delegation()
        .get_mandate(player_mandate)
        .expect("mandate record must persist");
    let prior_version = mandate_record.version();
    let mut revised_orders = mandate_record.standing_orders().clone();
    revised_orders.insert(
        PolicyKind::IndependentRecruitment,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    );
    validate_revise_mandate(
        registry,
        &scenario.state,
        player_mandate,
        MandateRevisionDraft {
            scopes: mandate_record.scopes().clone(),
            standing_orders: revised_orders,
            budget: mandate_record.budget(),
        },
    )?
    .commit(&mut scenario.state)?;
    let revised = scenario
        .state
        .delegation()
        .get_mandate(player_mandate)
        .expect("revised mandate must persist");
    if revised.version() <= prior_version {
        return Err("mandate revision did not advance version".into());
    }
    validate_harness_state(registry, &scenario.state)?;
    println!(
        "[CAPACITY] Mandate {:?} revised (v{} -> v{}) with updated standing orders, proving delegation lifecycle tracks the game.",
        player_mandate,
        prior_version,
        revised.version()
    );
    // Approach variation probe: authorize a WitnessPressure operation with a non-Covert
    // approach to prove the harness is not hard-coded to one tactical axis.
    let approach = match seed % 3 {
        0 => OperationApproach::Deceptive,
        1 => OperationApproach::Intimidating,
        _ => OperationApproach::Covert,
    };
    let _witness_pressure = validate_authorize_operation(
        registry,
        &scenario.state,
        OperationDraft {
            title: "capacity probe approach-variation operation".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: scenario.player,
            leader: scenario.boss,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(scenario.burglar),
            },
            approach,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.boss),
                (RoleKind::Negotiator, scenario.lieutenant),
            ]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: scenario.state.now() + SimDuration::from_minutes(5),
        },
    );
    // The operation may be rejected for domain reasons (e.g., witness not yet in case); the
    // probe's value is exercising a different OperationKind/Approach through the canonical
    // validation path, not asserting a specific operational outcome. A successful validation
    // proves the vocabulary is live; a typed rejection proves the harness tracks the game.
    validate_harness_state(registry, &scenario.state)?;
    println!(
        "[CAPACITY] Approach-variation probe exercised {:?} + {:?} through canonical validation.",
        OperationKind::WitnessPressure,
        approach
    );
    // Recruitment-approach variation: validate a non-FinancialOpportunity pitch through the
    // canonical path to prove the harness is not hard-coded to one approach. The probe uses
    // the same deterministic relationship so margin math stays registry-derived.
    let alt_approach = match seed % 4 {
        0 => RecruitmentApproach::FinancialOpportunity,
        1 => RecruitmentApproach::Advancement,
        2 => RecruitmentApproach::Protection,
        _ => RecruitmentApproach::PersonalAppeal,
    };
    let _alt_recruitment = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            recruiter: scenario.boss,
            candidate: scenario.danny_ferro,
            target_organization: scenario.player,
            approach: alt_approach,
        },
    );
    validate_harness_state(registry, &scenario.state)?;
    println!(
        "[CAPACITY] Recruitment-approach probe exercised {:?} through canonical validation.",
        alt_approach
    );
    Ok(())
}

fn capture_terminal_status(scenario: &Scenario, operation: OperationId, metrics: &mut RunMetrics) {
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("capacity-probe operation must remain queryable");
    metrics.aborted = record.status() == OperationStatus::Aborted;
    metrics.outcome = record
        .resolution()
        .map(|resolution| resolution.objective_outcome());
    if metrics.aborted {
        let abort = record
            .abort_record()
            .expect("aborted capacity-probe operation must retain its cause");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    }
}

fn run_until_operation_terminal(
    scenario: &mut Scenario,
    operation: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let started_at = scenario.state.now();
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("authorized operation must remain queryable");
    let operation_kind = record.kind();
    let authored_duration_minutes = scenario
        .registry
        .get_operation(operation_kind)
        .execution()
        .duration()
        .as_minutes();
    // The terminal-wait guard anchors at the later of the loop's current minute and the
    // operation's authored schedule, then adds the authored duration plus a decision and
    // police-arrival slack window, so it covers the wait-to-start and stays synchronized with
    // authored content instead of a hard-coded constant that could go stale.
    let guard_anchor = record.scheduled_for().max(started_at);
    let deadline = guard_anchor
        + SimDuration::from_minutes(authored_duration_minutes + OPERATION_WAIT_SLACK_MINUTES);
    loop {
        let status = scenario
            .state
            .operations()
            .get_operation(operation)
            .expect("authorized operation must remain queryable")
            .status();
        if matches!(
            status,
            OperationStatus::Completed | OperationStatus::Aborted
        ) {
            return Ok(());
        }
        if scenario.state.now() >= deadline {
            return Err(HarnessContractError::OperationDidNotTerminate {
                operation,
                started_at: started_at.as_minutes(),
                deadline: deadline.as_minutes(),
            }
            .into());
        }
        let outcome = run_tick(scenario.registry, &mut scenario.state);
        observe_tick(scenario, &outcome, narrative, metrics)?;
    }
}

fn run_until(
    scenario: &mut Scenario,
    until: SimTime,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    while scenario.state.now() < until {
        let outcome = run_tick(scenario.registry, &mut scenario.state);
        observe_tick(scenario, &outcome, narrative, metrics)?;
    }
    Ok(())
}

fn observe_tick(
    scenario: &mut Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    if narrative {
        for operation in &outcome.started_operations {
            let record = scenario
                .state
                .operations()
                .get_operation(*operation)
                .expect("started operation must exist");
            println!(
                "[START]   {}: {} started.",
                stamp(outcome.now.as_minutes()),
                record.title()
            );
            if let Some(response) = record.police_response() {
                let response = scenario
                    .state
                    .legal()
                    .get_police_response(response)
                    .expect("dispatched response must persist");
                println!(
                    "          Police response dispatched; estimated arrival minute {} based on local deployment.",
                    response.arrival_due_at().as_minutes()
                );
            }
        }
    }

    // A cold case shelved by its owning authority is a player-visible consequence resolution: the
    // organization can verify it later through its own surveillance, and the narrative prints the
    // institutional beat when the authored inactivity window elapses.
    if let Some(case) = scenario.investigation {
        if outcome.cold_case_suspensions.contains(&case) {
            metrics.case_cold_minute = Some(outcome.now.as_minutes());
            if narrative {
                let owner = scenario
                    .state
                    .legal()
                    .get_investigation(case)
                    .expect("shelved case must persist")
                    .owner();
                let owner_name = scenario
                    .state
                    .world()
                    .get_organization(owner)
                    .expect("case owner must persist")
                    .name();
                println!(
                    "[CASE COLD] {}: {} shelved the case after sustained routine investigation found no actionable subject.",
                    stamp(outcome.now.as_minutes()),
                    owner_name
                );
            }
        }
    }

    // The second score's lapse is a deliberate, observed consequence: PRESS stands down while the
    // case is hot and the opportunity expires through the canonical lifecycle, generating its own
    // report instead of any hidden-state read.
    if let Some(opportunity) = metrics.second_opportunity {
        if outcome.expired_opportunities.contains(&opportunity) {
            metrics.second_opportunity_expired = true;
            if narrative {
                println!(
                    "[OPPORTUNITY COST] {}: the second score on {} lapsed without action. The standing-down discipline that protects the hot case has a real price.",
                    stamp(outcome.now.as_minutes()),
                    scenario.variation.alternate_target_name()
                );
            }
        }
    }

    for operation in &outcome.started_operations {
        if Some(*operation) == metrics.burglary {
            metrics.police_dispatched = scenario
                .state
                .operations()
                .get_operation(*operation)
                .and_then(|record| record.police_response())
                .is_some();
        }
    }
    if let Some(burglary) = metrics.burglary {
        metrics.police_dispatched |= scenario
            .state
            .operations()
            .get_operation(burglary)
            .and_then(|record| record.police_response())
            .is_some();
        if let Some(response) = scenario
            .state
            .operations()
            .get_operation(burglary)
            .and_then(|record| record.police_response())
        {
            metrics.police_arrived |= outcome.arrived_police_responses.contains(&response);
        }
    }

    for request in &outcome.decision_requests {
        metrics.decision_requests += 1;
        let decision = scenario
            .state
            .decisions()
            .get_decision(request.decision)
            .expect("surfaced decision must persist");
        if narrative {
            println!(
                "[EXCEPTION] {}: {}",
                stamp(outcome.now.as_minutes()),
                decision.summary()
            );
        }
        let response = match decision.context() {
            DecisionContext::OperationException {
                reason: OperationExceptionReason::PoliceArrival(_),
                ..
            } if metrics.strategy == Some(Strategy::Press) => DecisionResponse::Continue,
            DecisionContext::OperationException {
                reason: OperationExceptionReason::PoliceArrival(_),
                ..
            } => DecisionResponse::Abort,
            DecisionContext::OperationException { .. } => DecisionResponse::Continue,
            DecisionContext::RecruitmentApproval(_) => DecisionResponse::Reject,
        };
        if narrative {
            println!("[DECIDE]  Leadership response: {response:?}.");
        }
        let resolution = validate_resolve_decision(
            scenario.registry,
            &scenario.state,
            request.decision,
            decision.recipient(),
            response,
        )?
        .commit(&mut scenario.state)?;
        if let Some(follow_up) = resolution.decision_request {
            let follow_up_record = scenario
                .state
                .decisions()
                .get_decision(follow_up.decision)
                .expect("follow-up decision must persist");
            // The deferred follow-up is itself a police-arrival decision, so it honors the same
            // branch policy as the decision it was queued behind: PRESS presses on, the other
            // strategies abort.
            let follow_up_response = match follow_up_record.context() {
                DecisionContext::OperationException {
                    reason: OperationExceptionReason::PoliceArrival(_),
                    ..
                } if metrics.strategy == Some(Strategy::Press) => DecisionResponse::Continue,
                // Every other deferred follow-up context (non-arrival exception, or any approval
                // context) is not produced on this path; abort defensively.
                DecisionContext::OperationException {
                    reason: OperationExceptionReason::PoliceArrival(_),
                    ..
                } => DecisionResponse::Abort,
                DecisionContext::OperationException { .. } => DecisionResponse::Abort,
                DecisionContext::RecruitmentApproval(_) => DecisionResponse::Abort,
            };
            metrics.decision_requests += 1;
            if narrative {
                println!("[EXCEPTION] Deferred: {}", follow_up_record.summary());
                println!("[DECIDE]  Leadership response: {follow_up_response:?}.");
            }
            validate_resolve_decision(
                scenario.registry,
                &scenario.state,
                follow_up.decision,
                follow_up_record.recipient(),
                follow_up_response,
            )?
            .commit(&mut scenario.state)?;
        }
    }

    // Press is the branch where the leader chooses to continue after police arrival. The
    // response also creates direct observations for the participating people; report those
    // observations through the canonical transfer path so the player-facing organization view
    // contains the lived consequence without reading hidden case state.
    if metrics.strategy == Some(Strategy::Press) && metrics.police_arrived {
        let sources: Vec<_> = scenario
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Character(scenario.burglar),
                InformationTopic::PoliceActivity,
            )
            .filter(|information| information.observed_at() == outcome.now)
            .map(|information| information.id())
            .collect();
        for source in sources {
            let already_reported = scenario
                .state
                .intelligence()
                .information_derived_from(source)
                .any(|information| {
                    information.holder() == KnowledgeHolder::Organization(scenario.player)
                });
            if already_reported {
                continue;
            }
            validate_information_transfer(
                &scenario.state,
                InformationTransferDraft {
                    source,
                    recipient: KnowledgeHolder::Organization(scenario.player),
                },
            )?
            .commit(&mut scenario.state)?;
            metrics.player_police_activity_information =
                metrics.player_police_activity_information.saturating_add(1);
            if narrative {
                println!(
                    "[PLAYER ACTION] {}: the crew reported the police response back to Marrow Organization; the organization now knows what the burglar directly experienced.",
                    stamp(outcome.now.as_minutes()),
                );
            }
        }
    }

    metrics.autonomous_recruitment_attempts = metrics
        .autonomous_recruitment_attempts
        .saturating_add(u32::try_from(outcome.recruitment_attempts.len()).unwrap_or(u32::MAX));
    for attempt in &outcome.recruitment_attempts {
        let attempt = scenario
            .state
            .recruitment()
            .get_attempt(*attempt)
            .expect("autonomous recruitment attempt must persist");
        if attempt.previous_organization() == Some(scenario.player)
            && attempt.outcome() == crimocracy::recruitment::RecruitmentOutcome::Accepted
        {
            metrics.player_personnel_departures =
                metrics.player_personnel_departures.saturating_add(1);
            metrics.defector = Some(attempt.candidate());
            metrics.defection_minute = Some(outcome.now.as_minutes());
        }
        if narrative {
            let recruiter = scenario
                .state
                .world()
                .get_character(attempt.recruiter())
                .expect("autonomous recruiter must exist");
            let candidate = scenario
                .state
                .world()
                .get_character(attempt.candidate())
                .expect("autonomous candidate must exist");
            println!(
                "[AUTONOMY] {}: {} independently approached {} using {:?}; pressure {}, margin {}, outcome {:?}.",
                stamp(outcome.now.as_minutes()),
                recruiter.name(),
                candidate.name(),
                attempt.approach(),
                attempt.factors().perceived_legal_pressure(),
                attempt.margin(),
                attempt.outcome(),
            );
            println!(
                "[DEV AUDIT] Recruitment factors: drive alignment {}, relationship support {}, incumbent attachment {}, incumbent resentment {}, perceived legal pressure {}, membership resistance {}, trait adjustment {}.",
                attempt.factors().drive_alignment(),
                attempt.factors().relationship_support(),
                attempt.factors().incumbent_attachment(),
                attempt.factors().incumbent_resentment(),
                attempt.factors().perceived_legal_pressure(),
                attempt.factors().membership_resistance(),
                attempt.factors().trait_adjustment(),
            );
            narrate_recruitment_causality(scenario, metrics, attempt, candidate);
        }
    }

    if narrative {
        for (investigation, investigator) in &outcome.staffed_investigations {
            let case = scenario
                .state
                .legal()
                .get_investigation(*investigation)
                .expect("staffed investigation must persist");
            let investigator = scenario
                .state
                .world()
                .get_character(*investigator)
                .expect("staffed investigator must persist");
            println!(
                "[DEV AUDIT] minute {:>4}: {} assigned {} as lead investigator.",
                outcome.now.as_minutes(),
                case.title(),
                investigator.name(),
            );
        }
        for work_id in &outcome.scheduled_investigation_work {
            let work = scenario
                .state
                .legal()
                .get_investigation_work(*work_id)
                .expect("scheduled investigation work must persist");
            let source = work
                .focus()
                .evidence_id()
                .and_then(|evidence| scenario.state.legal().get_evidence(evidence));
            println!(
                "[DEV AUDIT] minute {:>4}: scheduled {:?} due minute {} using {:?} evidence.",
                outcome.now.as_minutes(),
                work.kind(),
                work.due_at().as_minutes(),
                source.map(|evidence| evidence.kind()),
            );
        }
        for work_id in &outcome.resolved_investigation_work {
            let work = scenario
                .state
                .legal()
                .get_investigation_work(*work_id)
                .expect("resolved investigation work must persist");
            let resolution = work
                .resolution()
                .expect("resolved investigation work must have a resolution");
            let derived = resolution
                .derived_evidence()
                .and_then(|evidence| scenario.state.legal().get_evidence(evidence));
            println!(
                "[DEV AUDIT] minute {:>4}: {:?} resolved {:?} at margin {}; derived {:?}.",
                outcome.now.as_minutes(),
                work.kind(),
                resolution.outcome(),
                resolution.margin(),
                derived.map(|evidence| evidence.kind()),
            );
        }
    }
    metrics.investigation_work_scheduled = metrics.investigation_work_scheduled.saturating_add(
        u32::try_from(outcome.scheduled_investigation_work.len()).unwrap_or(u32::MAX),
    );
    metrics.investigation_work_resolved = metrics.investigation_work_resolved.saturating_add(
        u32::try_from(outcome.resolved_investigation_work.len()).unwrap_or(u32::MAX),
    );

    if narrative {
        for operation in &outcome.resolved_operations {
            let record = scenario
                .state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must persist");
            let resolution = record
                .resolution()
                .expect("resolved operation must have result");
            println!(
                "[RESULT]  {}: {} -> {:?}, exposure {:?}.",
                stamp(outcome.now.as_minutes()),
                record.title(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            );
        }
        if !outcome.business_cycles.is_empty() || !outcome.enterprise_cycles.is_empty() {
            println!(
                "[ROUTINE] {}: {} legitimate business cycle(s), {} delegated enterprise cycle(s).",
                stamp(outcome.now.as_minutes()),
                outcome.business_cycles.len(),
                outcome.enterprise_cycles.len(),
            );
        }
        if let Some(report) = outcome.executive_brief {
            let report = scenario
                .state
                .reports()
                .get_report(report)
                .expect("executive brief must persist");
            println!(
                "[BRIEF GENERATED] {}: executive brief with {} player-visible entr{}; full brief appears in the final recap.",
                stamp(outcome.now.as_minutes()),
                report.entries().len(),
                if report.entries().len() == 1 { "y" } else { "ies" },
            );
        }
    }
    if tick_changed_observable_state(outcome) {
        validate_harness_state(scenario.registry, &scenario.state)?;
    }
    Ok(())
}

/// Explains why an autonomous recruitment attempt landed or failed, connecting it to this
/// session's player-visible events. `[NARRATION]` is the harness's documentary voice: it may
/// explain world causality, but it never feeds action selection and never reads hidden
/// case/evidence state.
fn narrate_recruitment_causality(
    scenario: &Scenario,
    metrics: &RunMetrics,
    attempt: &crimocracy::recruitment::RecruitmentAttemptRecord,
    candidate: &crimocracy::world::CharacterRecord,
) {
    let candidate_name = candidate.name();
    let crew_role = metrics
        .burglary
        .and_then(|operation| scenario.state.operations().get_operation(operation))
        .and_then(|operation| {
            operation
                .roles()
                .iter()
                .find_map(|(role, member)| (*member == attempt.candidate()).then_some(*role))
        });
    let operation_title = metrics
        .burglary
        .and_then(|operation| scenario.state.operations().get_operation(operation))
        .map(|operation| operation.title().to_owned());
    let accepted = attempt.outcome() == crimocracy::recruitment::RecruitmentOutcome::Accepted;
    match (
        accepted,
        metrics.police_arrived,
        crew_role,
        operation_title,
    ) {
        (true, true, Some(role), Some(title)) => println!(
            "[NARRATION] {candidate_name} was on the {title} crew when police arrived. That direct contact is the lever a rival's {:?} pitch exploited; the organization loses its {} for burglary work.",
            attempt.approach(),
            role_label(role),
        ),
        (true, true, _, Some(title)) => println!(
            "[NARRATION] Police contact during the {title} operation opened the pressure window a rival's {:?} pitch exploited.",
            attempt.approach(),
        ),
        (true, _, _, _) => println!(
            "[NARRATION] {candidate_name} left the organization even without a revealed police contact this session; the rival's {:?} approach found another opening.",
            attempt.approach(),
        ),
        (false, false, _, _) => println!(
            "[NARRATION] {candidate_name} was not exposed to police this session, so the rival's {:?} pitch carried no immediate legal-pressure lever and it failed. The organization keeps its personnel.",
            attempt.approach(),
        ),
        (false, true, _, _) => println!(
            "[NARRATION] {candidate_name} refused the rival's {:?} pitch despite this session's police contact; the organization keeps its personnel.",
            attempt.approach(),
        ),
    }
}

fn role_label(role: RoleKind) -> &'static str {
    match role {
        RoleKind::Driver => "driver",
        RoleKind::Lookout => "lookout",
        RoleKind::EntrySpecialist => "entry specialist",
        RoleKind::SafeSpecialist => "safe specialist",
        RoleKind::Muscle => "muscle",
        RoleKind::InsideContact => "inside contact",
        RoleKind::Coordinator => "coordinator",
        RoleKind::Surveillance => "surveillance operator",
        RoleKind::Negotiator => "negotiator",
    }
}

/// Player-earned counter-intelligence after an accepted defection: the organization watches every
/// known rival through canonical surveillance to confirm where the departed member resurfaces. The
/// departure report deliberately never names the recruiting organization; this follow-up is the
/// player-visible channel that closes the knowledge loop without any hidden-state reads.
fn run_defector_trail(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(defector) = metrics.defector else {
        return Ok(());
    };
    let defector_name = scenario
        .state
        .world()
        .get_character(defector)
        .expect("departed character must persist")
        .name()
        .to_owned();
    let player_name = scenario
        .state
        .world()
        .get_organization(scenario.player)
        .expect("player organization must persist")
        .name()
        .to_owned();
    let first_rival_name = scenario
        .state
        .world()
        .get_organization(scenario.rival)
        .expect("rival organization must persist")
        .name()
        .to_owned();
    if narrative {
        let departed_at = metrics
            .defection_minute
            .map(|minute| format!(" at minute {minute}"))
            .unwrap_or_default();
        println!(
            "[DECIDE]  {player_name} knows {defector_name} left{departed_at}. Watch the district's known rivals for where a defector resurfaces; start with {first_rival_name}."
        );
    }
    // The fixture has two named rivals; watch each one through the player's own surveillance. At
    // most one rival is the true destination, so the trail confirms where the member landed while
    // still showing absence everywhere else through the same canonical channel.
    let known_rivals = [scenario.rival, scenario.second_rival];
    let mut resurfaced_at: Option<OrganizationId> = None;
    for rival in known_rivals {
        let rival_name = scenario
            .state
            .world()
            .get_organization(rival)
            .expect("known rival must persist")
            .name()
            .to_owned();
        let title = format!("{rival_name} personnel watch");
        let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
        let operation = authorize_surveillance_target(
            scenario,
            EntityRef::Organization(rival),
            &title,
            scheduled_for,
        )?;
        run_until_operation_terminal(scenario, operation, narrative, metrics)?;
        let resolution = scenario
            .state
            .operations()
            .get_operation(operation)
            .expect("personnel watch must persist")
            .resolution()
            .expect("completed personnel watch must have a resolution");
        let found = resolution
            .discovered_information()
            .iter()
            .any(|information| {
                scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .is_some_and(|record| {
                        record.topic() == InformationTopic::Personnel
                            && record.subject() == EntityRef::Organization(rival)
                            && record.summary().contains(&defector_name)
                    })
            });
        if found {
            resurfaced_at = Some(rival);
        }
        if narrative {
            for information in resolution.discovered_information() {
                let record = scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("personnel-watch information must persist");
                if record.topic() == InformationTopic::Personnel
                    && record.subject() == EntityRef::Organization(rival)
                {
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
            }
            if found {
                println!(
                    "[VERIFY DEFECTOR] {defector_name} now appears among {rival_name}'s recurring personnel."
                );
            } else {
                println!(
                    "[VERIFY DEFECTOR] {rival_name}'s watch shows {defector_name} is not working there."
                );
            }
        }
    }
    metrics.defector_trail_confirmed = Some(resurfaced_at.is_some());
    if narrative {
        match resurfaced_at {
            Some(rival) => {
                let rival_name = scenario
                    .state
                    .world()
                    .get_organization(rival)
                    .expect("confirmed rival must persist")
                    .name()
                    .to_owned();
                println!(
                    "[VERIFY DEFECTOR] {defector_name} resurfaces among {rival_name}'s personnel. {player_name} confirmed through its own surveillance where its former member landed."
                );
            }
            None => println!(
                "[VERIFY DEFECTOR] None of the watched rivals showed {defector_name}; the personnel watch did not directly confirm where the member landed."
            ),
        }
    }
    Ok(())
}

fn print_starting_player_view(scenario: &Scenario) {
    println!("[ORGANIZATION] Marrow Organization");
    for character in [
        scenario.boss,
        scenario.lieutenant,
        scenario.burglar,
        scenario.scout,
    ] {
        let record = scenario
            .state
            .world()
            .get_character(character)
            .expect("scenario character must exist");
        println!(
            "  - {:<14} autonomy {:?}; management {:?}; burglary {:?}; surveillance {:?}; stealth {:?}",
            record.name(),
            record.autonomy(),
            record.capability(CapabilityKind::Management).map(Rating::value),
            record.capability(CapabilityKind::Burglary).map(Rating::value),
            record.capability(CapabilityKind::Surveillance).map(Rating::value),
            record.capability(CapabilityKind::Stealth).map(Rating::value),
        );
    }
    println!(
        "[WORLD] In {}: player fronts {} and {}; the target is {}; {} holds jurisdiction; two rivals operate: {} and {}.",
        scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("neighborhood must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("front must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.resale_venue)
            .expect("resale venue must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.target)
            .expect("target must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.police)
            .expect("police must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.rival)
            .expect("rival must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.second_rival)
            .expect("second rival must exist")
            .name(),
    );
    println!(
        "[DELEGATION] Carlo manages a gambling enterprise at {}; routine cycles are delegated.",
        scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("front must exist")
            .name(),
    );
    let detective = scenario
        .state
        .world()
        .get_character(scenario.detective)
        .expect("detective must exist");
    println!(
        "[STATE] {} is available to Central Precinct with Investigation {}.",
        detective.name(),
        detective
            .capability(CapabilityKind::Investigation)
            .expect("detective must have investigation capability")
            .value(),
    );
    let replacement = scenario
        .state
        .world()
        .get_character(scenario.danny_ferro)
        .expect("replacement candidate must exist");
    println!(
        "[STATE] {} is an independent with Burglary {} / Stealth {}; Marrow holds a personal relationship with him, so he is the fallback entry specialist if the current crew is lost.",
        replacement.name(),
        replacement
            .capability(CapabilityKind::Burglary)
            .expect("replacement must have burglary capability")
            .value(),
        replacement
            .capability(CapabilityKind::Stealth)
            .expect("replacement must have stealth capability")
            .value(),
    );
}

fn print_planning_inputs(scenario: &Scenario, operation: OperationId) {
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("planning operation must persist");
    for information_id in record.intelligence() {
        let information = scenario
            .state
            .intelligence()
            .get_information(*information_id)
            .expect("selected planning information must persist");
        println!(
            "[PLAN INPUT] {:?} ({:?}/{:?}): {}",
            information.topic(),
            information.reliability(),
            information.specificity(),
            information.summary(),
        );
    }
}

fn print_resolution_factors(resolution: &crimocracy::operations::OperationResolutionRecord) {
    let factors = resolution.factors();
    println!(
        "[CAUSAL FACTORS] margin {}; crew {}; leader {:?}; intelligence {} (-{} difficulty, {}/{} areas); police {:?}; response {}; approach {}; time pressure {}; variance {}.",
        resolution.execution_margin(),
        factors.role_capability_average().value(),
        factors.leader_capability().map(Rating::value),
        factors.intelligence_quality().value(),
        factors.intelligence_adjustment().unsigned_abs(),
        factors.intelligence_topics_covered(),
        factors.intelligence_topics_relevant(),
        factors.target_police_presence().map(Rating::value),
        factors.police_response_arrived(),
        factors.approach_adjustment(),
        factors.time_pressure(),
        factors.variance(),
    );
}

fn print_player_knowledge_gap(scenario: &Scenario, burglary: OperationId) {
    let operation = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must persist");
    if let Some(resolution) = operation.resolution() {
        let legal_information: Vec<_> = scenario
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(scenario.player),
                InformationTopic::LegalActivity,
            )
            .filter(|information| information.subject() == EntityRef::Operation(burglary))
            .collect();
        println!(
            "[KNOWLEDGE] Player organization has {} LegalActivity information record(s) about this burglary after resolution.",
            legal_information.len(),
        );
        for information in legal_information {
            println!("  - [PLAYER] {}", information.summary());
        }
        if let Some(investigation) = resolution.exposure().investigation() {
            let hidden = scenario
                .state
                .legal()
                .get_investigation(investigation)
                .expect("exposure-linked investigation must exist");
            let lead = hidden
                .lead_investigator()
                .and_then(|lead| scenario.state.world().get_character(lead))
                .map(|record| record.name());
            let scheduled_work = scenario
                .state
                .legal()
                .work_for_investigation(investigation)
                .filter(|work| work.status() == InvestigationWorkStatus::Scheduled)
                .count();
            let completed_work = scenario
                .state
                .legal()
                .work_for_investigation(investigation)
                .filter(|work| work.status() == InvestigationWorkStatus::Completed)
                .count();
            println!(
                "[DEV AUDIT] Hidden state has case '{}' with {} subject(s), {} evidence item(s), lead {:?}, {} scheduled and {} completed detective work item(s).",
                hidden.title(),
                hidden.subjects().len(),
                hidden.evidence().len(),
                lead,
                scheduled_work,
                completed_work,
            );
        }
    }
}

fn print_final_case_audit(scenario: &Scenario, burglary: OperationId) {
    let Some(investigation) = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.exposure().investigation())
    else {
        return;
    };
    let case = scenario
        .state
        .legal()
        .get_investigation(investigation)
        .expect("exposure-linked investigation must persist");
    let evidence_kinds = case
        .evidence()
        .iter()
        .filter_map(|evidence| scenario.state.legal().get_evidence(*evidence))
        .map(|evidence| evidence.kind())
        .collect::<Vec<_>>();
    let work = scenario
        .state
        .legal()
        .work_for_investigation(investigation)
        .map(|work| {
            (
                work.kind(),
                work.status(),
                work.resolution().map(|resolution| resolution.outcome()),
            )
        })
        .collect::<Vec<_>>();
    println!(
        "\n[DEV AUDIT] Final hidden case state: {} subject(s), evidence {:?}, detective work {:?}.",
        case.subjects().len(),
        evidence_kinds,
        work,
    );
}

#[derive(Clone, Copy)]
struct FinancialView {
    legitimate_cycle_count: u32,
    legitimate_net_cents: i64,
    enterprise_cycle_count: usize,
    enterprise_net_cents: i64,
    street_cash_cents: i64,
    liquidation_cash_cents: i64,
    held_property_operations: u32,
    held_property_value_cents: i64,
    liquidated_property_operations: u32,
    liquidated_property_cash_cents: i64,
}

fn resolve_financial_view(scenario: &Scenario) -> Result<FinancialView, Box<dyn Error>> {
    let business_summary = resolve_organization_business_financial_summary(
        &scenario.state,
        scenario.player,
        SimTime::ZERO,
        scenario.state.now(),
    )?;
    let enterprise = scenario
        .state
        .enterprises()
        .get_enterprise(scenario.enterprise)
        .expect("scenario enterprise must exist");
    let enterprise_net = scenario
        .state
        .enterprises()
        .cycles_for(scenario.enterprise)
        .try_fold(Money::ZERO, |sum, cycle| sum.checked_add(cycle.net_cash()))
        .expect("scenario enterprise totals must fit money range");
    let street_cash = scenario
        .state
        .finance()
        .get_account(enterprise.cash_account())
        .expect("enterprise cash account must exist")
        .balance();
    let liquidation_cash = scenario
        .state
        .finance()
        .get_account(scenario.liquidation_cash)
        .expect("liquidation cash account must exist")
        .balance();
    let (held_property_operations, held_property_value) = scenario
        .state
        .operations()
        .operations_for_organization(scenario.player)
        .filter(|operation| operation.property_disposition().is_none())
        .filter_map(|operation| operation.resolution())
        .filter_map(|resolution| resolution.property_proceeds())
        .try_fold((0_u32, Money::ZERO), |(count, total), proceeds| {
            Some((
                count.checked_add(1)?,
                total.checked_add(proceeds.estimated_value())?,
            ))
        })
        .expect("scenario held-property totals must fit numeric bounds");
    let (liquidated_property_operations, liquidated_property_cash) = scenario
        .state
        .operations()
        .operations_for_organization(scenario.player)
        .filter_map(|operation| operation.property_disposition())
        .try_fold((0_u32, Money::ZERO), |(count, total), disposition| {
            Some((
                count.checked_add(1)?,
                total.checked_add(disposition.realized_value())?,
            ))
        })
        .expect("scenario liquidated-property totals must fit numeric bounds");
    Ok(FinancialView {
        legitimate_cycle_count: business_summary.totals.cycle_count,
        legitimate_net_cents: business_summary.totals.net_cash.cents(),
        enterprise_cycle_count: scenario
            .state
            .enterprises()
            .cycles_for(scenario.enterprise)
            .count(),
        enterprise_net_cents: enterprise_net.cents(),
        street_cash_cents: street_cash.cents(),
        liquidation_cash_cents: liquidation_cash.cents(),
        held_property_operations,
        held_property_value_cents: held_property_value.cents(),
        liquidated_property_operations,
        liquidated_property_cash_cents: liquidated_property_cash.cents(),
    })
}

fn print_financial_view(scenario: &Scenario, view: FinancialView) {
    println!(
        "\n[FINANCIAL VIEW {}]",
        stamp(scenario.state.now().as_minutes())
    );
    println!(
        "  Legitimate front: {} cycle(s), net {}.",
        view.legitimate_cycle_count,
        format_cents(view.legitimate_net_cents),
    );
    println!(
        "  Delegated gambling: {} cycle(s), net {}, street-cash balance {}.",
        view.enterprise_cycle_count,
        format_cents(view.enterprise_net_cents),
        format_cents(view.street_cash_cents),
    );
    println!(
        "  Resale liquidation cash balance: {}.",
        format_cents(view.liquidation_cash_cents),
    );
    println!(
        "  Held operation property: {} operation(s), estimated value {}, unliquidated.",
        view.held_property_operations,
        format_cents(view.held_property_value_cents),
    );
    println!(
        "  Liquidated operation property: {} disposition(s), realized {}.",
        view.liquidated_property_operations,
        format_cents(view.liquidated_property_cash_cents),
    );
}

fn print_report(label: &str, report: &ReportRecord, scenario: &Scenario) {
    println!(
        "[{label}] minute {}: {}",
        report.generated_at().as_minutes(),
        report.title()
    );
    for entry in report.entries() {
        let marker = match entry.attention {
            AttentionClass::Routine => "routine",
            AttentionClass::Notable => "notable",
            AttentionClass::Exception => "EXCEPTION",
            AttentionClass::Crisis => "CRISIS",
        };
        let context = entry.entities.iter().find_map(|entity| {
            if let EntityRef::Operation(operation) = entity {
                return scenario
                    .state
                    .operations()
                    .get_operation(*operation)
                    .map(|record| format!("operation: {}", record.title()));
            }
            None
        });
        if let Some(context) = context {
            println!("  - [{marker}] [{context}] {}", entry.summary);
        } else {
            println!("  - [{marker}] {}", entry.summary);
        }
    }
}

/// Condensed report rendering for routine briefs: header plus only the entries that need a
/// leader's attention. Full after-action text stays on the [AFTER-ACTION]/[ABORT REPORT] beats so
/// the interesting consequence text is not drowned in repeated boilerplate.
fn print_report_condensed(label: &str, report: &ReportRecord) {
    let entries = report.entries();
    println!(
        "[{label}] minute {}: {} ({} entries)",
        report.generated_at().as_minutes(),
        report.title(),
        entries.len()
    );
    for entry in entries {
        let marker = match entry.attention {
            AttentionClass::Routine => "routine",
            AttentionClass::Notable => "notable",
            AttentionClass::Exception => "EXCEPTION",
            AttentionClass::Crisis => "CRISIS",
        };
        if matches!(
            entry.attention,
            AttentionClass::Notable | AttentionClass::Exception | AttentionClass::Crisis
        ) {
            println!("  - [{marker}] {}", entry.summary);
        }
    }
}

fn print_metrics(metrics: &RunMetrics) {
    let property_acquired = optional_cents(metrics.property_acquired_value_cents);
    let property_realized = optional_cents(metrics.property_realized_cash_cents);
    let liquidation_minute = optional_minute(metrics.liquidation_minute);
    println!(
        "{:<6} [{:<9}]: {}, finish {:?}m, police dispatched {}, police arrived {}, decisions {}, plan items {} {:?}, intel {:?}, exposure {:?}/{:?}, property {} -> {} cash at {}, case {}, evidence {}, player legal intel {}, police intel {}, follow-up {:?}/{} info (case hot {:?}), cold confirmed {:?} @ {:?}, case work {}/{}, surveillance discoveries {}, reports {}, briefs {}, recruitment {}, departures {}, legit {}, enterprise {}",
        metrics.strategy.expect("strategy must be set").label(),
        metrics.variation.expect("fixture variation must be set").label(),
        terminal_label(metrics),
        metrics.burglary_terminal_minute,
        metrics.police_dispatched,
        metrics.police_arrived,
        metrics.decision_requests,
        metrics.planning_information_count,
        metrics.planning_information_topics,
        metrics.burglary_information_quality,
        metrics.exposure_level,
        metrics.exposure_score,
        property_acquired,
        property_realized,
        liquidation_minute,
        metrics.investigation_created,
        metrics.evidence_count,
        metrics.player_legal_activity_information,
        metrics.player_police_activity_information,
        metrics.counterintelligence_outcome,
        metrics.counterintelligence_information,
        metrics.followup_case_active,
        metrics.cold_case_confirmed,
        metrics.case_cold_minute,
        metrics.investigation_work_scheduled,
        metrics.investigation_work_resolved,
        metrics.discovered_surveillance_information,
        metrics.player_report_count,
        metrics.executive_brief_count,
        metrics.autonomous_recruitment_attempts,
        metrics.player_personnel_departures,
        optional_cents(metrics.legitimate_net_cents),
        optional_cents(metrics.enterprise_net_cents),
    );
    println!(
        "        act 2: second score discovered {}, expired {}, replacement {}, second burglary {} @ {} (outcome {:?}, aborted {}), recon info {}, property {} -> {}",
        metrics.second_opportunity_discovered,
        metrics.second_opportunity_expired,
        metrics.replacement_recruited,
        metrics.second_burglary.is_some(),
        optional_minute(metrics.second_burglary_terminal_minute),
        metrics.second_burglary_outcome,
        metrics.second_burglary_aborted,
        metrics.second_act_recon_information,
        optional_cents(metrics.second_act_property_acquired_value_cents),
        optional_cents(metrics.second_act_property_realized_cash_cents),
    );
}

fn optional_cents(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), |cents| format!("{cents}c"))
}

fn optional_minute(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |minute| format!("{minute}m"))
}

fn print_experience_readout(rush: &RunMetrics, press: &RunMetrics, recon: &RunMetrics) {
    println!("\n--- PLAYER LOOP READOUT ---");
    println!(
        "The core fantasy tested here is: learn what the city reveals, turn it into an organizational plan, delegate execution, then stay powerful enough to absorb the consequences."
    );
    println!("Evidence coverage (not a game-quality score):");
    print_loop_checkpoint(
        "learn",
        recon.discovered_surveillance_information > 0,
        "surveillance produces actionable patrol and target information",
    );
    print_loop_checkpoint(
        "plan",
        recon.planning_information_count > rush.planning_information_count
            && recon.burglary_information_quality.unwrap_or_default()
                > rush.burglary_information_quality.unwrap_or_default(),
        "the player can make a better plan from organization-held intelligence",
    );
    let response_choice_changed_consequence = rush.aborted
        && press.outcome.is_some()
        && press.decision_requests > 0
        && press.player_police_activity_information > 0;
    print_loop_checkpoint(
        "choice",
        response_choice_changed_consequence,
        "a player response to the same police exception changes whether the operation aborts or resolves",
    );
    print_loop_checkpoint(
        "delegate",
        recon.burglary.is_some() && recon.outcome.is_some(),
        "the plan resolves through assigned people and authored capabilities",
    );
    print_loop_checkpoint(
        "respond",
        press.decision_requests > 0 && press.player_police_activity_information > 0,
        "an exception pauses the plan and a field report returns to the organization",
    );
    print_loop_checkpoint(
        "consequences",
        press.investigation_created && recon.property_realized_cash_cents.is_some(),
        "the same operation system can create legal pressure or recover value into cash",
    );
    print_loop_checkpoint(
        "follow-up",
        press.counterintelligence_outcome.is_some()
            && press.counterintelligence_information > 0
            && press.followup_case_active == Some(true),
        "a player-visible legal report can seed a precinct check that reads whether the case is still hot",
    );
    print_loop_checkpoint(
        "survive",
        press.cold_case_confirmed == Some(true) && press.case_cold_minute.is_some(),
        "standing down and outlasting the investigation resolves the consequence through the player's own surveillance",
    );
    print_loop_checkpoint(
        "organization",
        rush.autonomous_recruitment_attempts > 0 && rush.player_personnel_departures > 0,
        "a police-exposed crew member can be courted away by a rival without a scripted event",
    );
    print_loop_checkpoint(
        "rebuild",
        rush.replacement_recruited && rush.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved),
        "a crew member lost to rival pressure can be replaced through a player-authored executive recruitment, and the rebuilt crew works a second score safely",
    );
    print_loop_checkpoint(
        "second wind",
        recon.second_act_recon_information > 0
            && recon.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved),
        "an organization that re-invests in planning can recover value on a reopened window",
    );
    print_loop_checkpoint(
        "discipline cost",
        press.second_opportunity_expired && press.second_burglary.is_none(),
        "choosing to stand down has a real price: the second score lapses while the hot case stays protected",
    );
    let defector_trail_shown = rush.defector_trail_confirmed == Some(true)
        && press.defector_trail_confirmed == Some(true)
        && recon.defector_trail_confirmed.is_none();
    print_loop_checkpoint(
        "defector trail",
        defector_trail_shown,
        "after a departure, the organization can confirm where the defector landed through its own canonical surveillance channel instead of the report leaking the rival",
    );
    let legitimate_isolated = rush.legitimate_net_cents == press.legitimate_net_cents
        && press.legitimate_net_cents == recon.legitimate_net_cents;
    let enterprise_heat_shown = press.enterprise_net_cents.unwrap_or(0)
        < recon.enterprise_net_cents.unwrap_or(0)
        && press.investigation_created
        && !recon.investigation_created;
    print_loop_checkpoint(
        "routine",
        legitimate_isolated,
        "legitimate front continues identically while leadership focuses on exceptions",
    );
    print_loop_checkpoint(
        "heat cost",
        enterprise_heat_shown,
        "an active investigation in the district raises the delegated enterprise's operating cost",
    );
    let liquidation_varies = rush
        .second_act_property_realized_cash_cents
        .zip(recon.second_act_property_realized_cash_cents)
        .map(|(a, b)| a != b)
        .unwrap_or(false)
        || recon.property_realized_cash_cents.is_some()
            && press.property_realized_cash_cents.is_none();
    print_loop_checkpoint(
        "venue choice",
        liquidation_varies || recon.property_realized_cash_cents.is_some(),
        "liquidated resale value reflects the venue's district police presence",
    );
    println!("Observed decision leverage:");
    println!(
        "  - Information leverage: RECON selected {} planning item(s) versus RUSH's {} and finished as {} versus {}.",
        recon.planning_information_count,
        rush.planning_information_count,
        terminal_label(recon),
        terminal_label(rush),
    );
    println!(
        "  - Exception leverage: PRESS chose Continue at {} surfaced decision(s); RUSH chose Abort through its standing contingency, producing {} versus {}.",
        press.decision_requests,
        terminal_label(press),
        terminal_label(rush),
    );
    println!(
        "  - Personnel leverage: RUSH/PRESS exposed the crew to police and lost {} crew member(s) to rival recruitment, while RECON kept everyone ({} departures) because the crew never saw police.",
        rush.player_personnel_departures + press.player_personnel_departures,
        recon.player_personnel_departures,
    );
    println!(
        "  - Consequence leverage: PRESS exposed {} evidence item(s), {} legal-activity information item(s), read the case as still hot at minute ~{}, then confirmed it shelved at minute {}; enterprise heat cut gambling net to {} vs RECON {} while hot, then recovered after the shelf; RECON realized {} of resale cash via a low-police venue.",
        press.evidence_count,
        press.player_legal_activity_information,
        press.counterintelligence_scheduled_at.unwrap_or_default(),
        press.case_cold_minute.unwrap_or_default(),
        optional_dollars(press.enterprise_net_cents),
        optional_dollars(recon.enterprise_net_cents),
        optional_dollars(recon.property_realized_cash_cents),
    );
    println!(
        "  - Time tradeoff: RECON finished at minute {} versus RUSH at minute {}; the extra planning time bought lower exposure and liquid value in this matched fixture.",
        recon.burglary_terminal_minute.unwrap_or_default(),
        rush.burglary_terminal_minute.unwrap_or_default(),
    );
    println!("Current experience gaps exposed by this fixture:");
    println!(
        "  - The consequence arc now closes and bleeds into economics: an open case can be read, outlasted, verified shelved, and while hot it raises the delegated enterprise's heat surcharge and reduces resale value in heavily patrolled districts. Disrupting evidence, influencing counsel, or changing a prosecution outcome are still not modeled."
    );
    println!(
        "  - The portfolio probe covers prioritization and expiry across competing opportunities, while the organizational-capacity probe now proves overlapping specialist assignments reject atomically and release after completion, plus mandate revision and approach variation. Broader resource competition and rival-initiated enterprise targeting remain outside this foundation."
    );
    println!(
        "  - A defector's destination is discoverable only through the player's own surveillance watch; there is still no modeled way to pre-empt a defection, win a member back, or retaliate. The fixture's second rival (D'Amato Crew) is watched to confirm absence but makes no autonomous moves of its own yet."
    );
    println!(
        "  - The delegation pillar is now measurably hot: enterprise heat is visible, but the fixtures still never ask the player to re-scope a mandate, replace a delegated manager mid-crisis, or respond to manager drift beyond the capacity-probe revision."
    );
    println!(
        "  - The RUSH/PRESS/RECON policies are calibration treatments; each matched seed shares one authored-content-derived timeline while bounded policy offsets vary the act-1 and second-wind clock choices. They are not evidence that an actual player would choose the same policies or the same rebuild/second-wind scheduling."
    );
}

fn print_loop_checkpoint(label: &str, present: bool, evidence: &str) {
    println!(
        "  [{:>12}] {:<5} - {}",
        label,
        if present { "shown" } else { "missing" },
        evidence,
    );
}

fn terminal_label(metrics: &RunMetrics) -> String {
    if metrics.aborted {
        format!(
            "aborted {:?} / {:?}",
            metrics.abort_phase, metrics.abort_cause
        )
    } else {
        format!("completed {:?}", metrics.outcome)
    }
}

/// Renders an absolute campaign minute as the clock time the player would see on a report.
fn format_minute_of_day(minute: u64) -> String {
    let minute_of_day = minute % 1_440;
    format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// Renders a player-facing tick beat as minute plus clock, e.g. `minute 160 (02:40)`.
fn stamp(minute: u64) -> String {
    format!("minute {} ({})", minute, format_minute_of_day(minute))
}

/// Renders cents as a player-facing dollar amount, e.g. `23019` -> `$230.19`.
fn format_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let magnitude = cents.unsigned_abs();
    format!("{sign}${}.{:02}", magnitude / 100, magnitude % 100)
}

fn optional_dollars(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), format_cents)
}

/// True when the tick produced any transaction a player could observe or that persists state.
/// The harness validates the whole world at these consequential boundaries; skipping fully routine
/// minutes keeps the matched-batch lane fast without losing corruption coverage at any real event.
fn tick_changed_observable_state(outcome: &TickOutcome) -> bool {
    !outcome.started_operations.is_empty()
        || !outcome.arrived_police_responses.is_empty()
        || !outcome.decision_requests.is_empty()
        || !outcome.resolved_operations.is_empty()
        || !outcome.staffed_investigations.is_empty()
        || !outcome.scheduled_investigation_work.is_empty()
        || !outcome.resolved_investigation_work.is_empty()
        || !outcome.business_cycles.is_empty()
        || !outcome.enterprise_cycles.is_empty()
        || !outcome.recruitment_attempts.is_empty()
        || !outcome.expired_opportunities.is_empty()
        || !outcome.cold_case_suspensions.is_empty()
        || outcome.executive_brief.is_some()
}

fn choose_safe_start_from_patrol_report(
    now: SimTime,
    report: &str,
    operation_duration: SimDuration,
    uncertainty_buffer: SimDuration,
    latest_start: SimTime,
) -> Result<SimTime, HarnessContractError> {
    let windows = parse_patrol_windows(report);
    if windows.is_empty() {
        return Err(HarnessContractError::NoActionablePatrolWindows);
    }
    let duration = u64::from(operation_duration.as_minutes());
    let buffer = u64::from(uncertainty_buffer.as_minutes());
    let earliest = now.as_minutes().saturating_add(1);
    let latest = latest_start.as_minutes().saturating_sub(duration);
    let first_candidate = earliest.div_ceil(30).saturating_mul(30);
    for candidate in (first_candidate..=latest)
        .step_by(30)
        .take_while(|candidate| *candidate < first_candidate.saturating_add(2_880))
    {
        let operation_start = candidate % 1_440;
        let operation_end = operation_start.saturating_add(duration);
        if operation_end > 1_440 {
            continue;
        }
        let overlaps_buffered_patrol = windows.iter().any(|(start, end)| {
            let buffered_start = start.saturating_sub(buffer);
            let buffered_end = end.saturating_add(buffer).min(1_440);
            intervals_overlap(operation_start, operation_end, buffered_start, buffered_end)
        });
        if !overlaps_buffered_patrol {
            return Ok(SimTime::from_minutes(candidate));
        }
    }
    Err(HarnessContractError::NoSafeOperationWindow)
}

fn parse_patrol_windows(report: &str) -> Vec<(u64, u64)> {
    let mut windows = Vec::new();
    let mut remaining = report;
    while let Some(index) = remaining.find("roughly ") {
        remaining = &remaining[index + "roughly ".len()..];
        let Some(start) = remaining.get(0..5).and_then(parse_clock_minute) else {
            break;
        };
        if remaining.get(5..6) != Some("-") {
            break;
        }
        let Some(end) = remaining.get(6..11).and_then(parse_clock_minute) else {
            break;
        };
        if start < end {
            windows.push((start, end));
        } else if start > end {
            windows.push((start, 1_440));
            if end > 0 {
                windows.push((0, end));
            }
        } else {
            windows.push((0, 1_440));
        }
        remaining = remaining.get(11..).unwrap_or_default();
    }
    windows
}

fn parse_clock_minute(value: &str) -> Option<u64> {
    let (hour, minute) = value.split_once(':')?;
    let hour = hour.parse::<u64>().ok()?;
    let minute = minute.parse::<u64>().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

fn intervals_overlap(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> bool {
    start_a < end_b && start_b < end_a
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("gameplay harness ratings are authored within 0..=100")
}

fn jitter_rating_u8(base: u8, jitter: i16) -> u8 {
    (i16::from(base) + jitter).clamp(0, 100) as u8
}

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value)
        .expect("gameplay harness relationship levels are authored within 0..=100")
}

#[cfg(test)]
mod tests {
    use super::{
        choose_safe_start_from_patrol_report, parse_options, parse_patrol_windows,
        run_opportunity_portfolio_probe, run_smoke, FixtureVariation, HarnessCliError,
        HarnessContractError, HarnessMode, HarnessOptions, ScenarioProfile, ScenarioTimeline,
        Strategy, DEFAULT_SEED,
    };
    use crimocracy::core::time::{SimDuration, SimTime};

    #[test]
    fn parses_explicit_smoke_mode_and_hex_seed() {
        let options = parse_options(
            ["--mode", "smoke", "--samples", "1", "--seed", "0x2a"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("valid harness arguments should parse")
        .expect("non-help arguments should request a run");

        assert_eq!(
            options,
            HarnessOptions {
                mode: HarnessMode::Smoke,
                samples: 1,
                seed: 42,
                strategy: None,
                artifact_dir: None,
            }
        );
    }

    #[test]
    fn accepts_uppercase_hex_prefix() {
        let options = parse_options(["--seed", "0X2A"].into_iter().map(str::to_owned))
            .expect("uppercase hexadecimal prefixes should parse")
            .expect("non-help arguments should request a run");

        assert_eq!(options.seed, 42);
    }

    #[test]
    fn parses_a_focused_smoke_strategy() {
        let options = parse_options(
            ["--mode", "smoke", "--strategy", "press"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("focused smoke arguments should parse")
        .expect("non-help arguments should request a run");

        assert_eq!(options.strategy, Some(Strategy::Press));
    }

    #[test]
    fn rejects_strategy_selection_in_full_mode() {
        let error = parse_options(
            ["--mode", "full", "--strategy", "press"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("full mode must keep all strategy branches matched");

        assert!(matches!(error, HarnessCliError::StrategyOnlyInSmoke));
    }

    #[test]
    fn uses_fast_smoke_mode_defaults() {
        let options = parse_options(std::iter::empty())
            .expect("default arguments should parse")
            .expect("default arguments should request a run");

        assert_eq!(options.mode, HarnessMode::Smoke);
        assert_eq!(options.samples, 1);
        assert_eq!(options.seed, DEFAULT_SEED);
    }

    #[test]
    fn keeps_full_mode_bounded_by_default() {
        let options = parse_options(["--mode", "full"].into_iter().map(str::to_owned))
            .expect("explicit full mode should parse")
            .expect("non-help arguments should request a run");

        assert_eq!(options.mode, HarnessMode::Full);
        assert_eq!(options.samples, super::DEFAULT_BATCH_SAMPLES);
        assert_eq!(options.samples, super::MIN_SAMPLES_FOR_VARIATION_CONTRACT);
    }

    #[test]
    fn rejects_out_of_range_sample_count() {
        let error = parse_options(["--samples", "0"].into_iter().map(str::to_owned))
            .expect_err("zero samples must be rejected");

        assert!(matches!(
            error,
            HarnessCliError::SampleCountOutOfRange { value: 0 }
        ));
    }

    #[test]
    fn rejects_multi_sample_smoke_mode() {
        let error = parse_options(
            ["--mode", "smoke", "--samples", "2"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("smoke mode must not silently ignore a larger batch request");

        assert!(matches!(
            error,
            HarnessCliError::SmokeSampleCount { value: 2 }
        ));
    }

    #[test]
    fn rejects_unknown_strategy() {
        let error = parse_options(
            ["--mode", "smoke", "--strategy", "reckon"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("unknown strategies must fail clearly");

        assert!(matches!(
            error,
            HarnessCliError::InvalidStrategy { value } if value == "reckon"
        ));
    }

    #[test]
    fn parses_wrapped_patrol_windows_without_empty_intervals() {
        assert_eq!(
            parse_patrol_windows(
                "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy)."
            ),
            vec![(120, 240), (1_320, 1_440)]
        );
    }

    #[test]
    fn chooses_a_buffered_window_from_player_visible_patrol_text() {
        let chosen = choose_safe_start_from_patrol_report(
            SimTime::from_minutes(1),
            "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy).",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(720),
        )
        .expect("actionable patrol text should produce a safe candidate");

        assert_eq!(chosen, SimTime::from_minutes(300));
    }

    #[test]
    fn rejects_patrol_text_without_actionable_windows() {
        let error = choose_safe_start_from_patrol_report(
            SimTime::ZERO,
            "Patrol activity was observed, but no recurring window was established.",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(720),
        )
        .expect_err("the harness must not infer a safe time from vague surveillance");

        assert!(matches!(
            error,
            HarnessContractError::NoActionablePatrolWindows
        ));
    }

    #[test]
    fn refuses_a_patrol_safe_start_after_opportunity_expiry() {
        let error = choose_safe_start_from_patrol_report(
            SimTime::from_minutes(1),
            "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy).",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(200),
        )
        .expect_err("planning must respect the player-visible opportunity deadline");

        assert!(matches!(error, HarnessContractError::NoSafeOperationWindow));
    }

    #[test]
    fn portfolio_probe_requires_explicit_opportunity_prioritization() {
        run_opportunity_portfolio_probe(&crimocracy::build_registry(), DEFAULT_SEED)
            .expect("portfolio probe should preserve selected and expired opportunities");
    }

    #[test]
    fn seed_selects_distinct_authored_fixture_variations() {
        let clockwork = FixtureVariation::from_seed(0);
        let crowded = FixtureVariation::from_seed(1);
        let quiet = FixtureVariation::from_seed(2);

        assert_ne!(clockwork, crowded);
        assert_ne!(crowded, quiet);
        assert_ne!(clockwork, quiet);
        assert_ne!(
            clockwork.patrol_windows(ScenarioProfile::NightTrap),
            crowded.patrol_windows(ScenarioProfile::NightTrap),
        );
        assert_ne!(clockwork.target_name(), crowded.target_name());
        assert_ne!(crowded.source_specificity(), quiet.source_specificity());
        assert_ne!(
            clockwork.neighborhood_economy(),
            quiet.neighborhood_economy(),
        );
    }

    #[test]
    fn scenario_timeline_is_seed_varied_and_registry_anchored() {
        let registry = crimocracy::build_registry();
        let first = ScenarioTimeline::for_scenario(&registry, 0);
        let matched = ScenarioTimeline::for_scenario(&registry, 0);
        let varied = ScenarioTimeline::for_scenario(&registry, 3);
        let burglary_duration = registry
            .get_operation(crimocracy::operations::OperationKind::Burglary)
            .execution()
            .duration();

        assert_eq!(first, matched, "matched branches must share one timeline");
        assert_ne!(
            first, varied,
            "batch policy timing must not replay one exact clock sequence"
        );
        assert!(first.initial_burglary_at < first.initial_opportunity_valid_until);
        assert!(
            first.rush_second_act_at + burglary_duration < first.second_opportunity_valid_until,
            "the authored operation must fit inside the derived second-score window"
        );
        assert!(
            first.recon_second_act_surveillance_at > first.second_opportunity_discovery_at,
            "fresh recon must follow the player-visible opportunity discovery"
        );
    }

    #[test]
    #[ignore = "controlled smoke contract runs in its focused local gate lane"]
    fn smoke_mode_covers_canonical_paths() {
        run_smoke(DEFAULT_SEED, None)
            .expect("smoke harness should pass its canonical-path contract");
    }
}
