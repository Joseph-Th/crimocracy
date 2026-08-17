//! Controlled/calibration harness for deterministic strategy and integration evidence.
//!
//! RUSH, PRESS, and RECON use canonical production operations and player-visible information.
//! `[DEV AUDIT]` output may inspect hidden state after decisions for diagnostics, but hidden state
//! never feeds action selection. Batch runs vary the simulation seed only; strategy policy is
//! deterministic and matched branches use the same seed.

use crimocracy::build_registry;
use crimocracy::core::attention::AttentionClass;
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{
    BusinessId, CharacterId, EnterpriseId, InformationId, OperationId, OrganizationId,
};
use crimocracy::core::invariants::{validate_state, validate_state_against_registry};
use crimocracy::core::simulation::{run_tick, TickOutcome};
use crimocracy::core::state::AppState;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::decisions::{DecisionContext, DecisionResponse, OperationExceptionReason};
use crimocracy::delegation::delegation_system::validate_assign_mandate;
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
    EvidenceStrength, InvestigationDraft, InvestigationWorkStatus, JurisdictionDraft,
    LegalRepresentationDraft, PatrolDeploymentDraft, PatrolWindow, ProsecutionCaseDraft,
};
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::property_disposition::{
    validate_dispose_property, PropertyDispositionDraft,
};
use crimocracy::operations::{
    OperationAbortCause, OperationAbortPhase, OperationApproach, OperationConstraint,
    OperationContingency, OperationDraft, OperationKind, OperationObjective,
    OperationObjectiveOutcome, OperationStatus, RoleKind,
};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
};
use crimocracy::opportunities::{OperationOpportunityDraft, OpportunityStatus};
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

const DEFAULT_BATCH_SAMPLES: u64 = 3;
const MAX_BATCH_SAMPLES: u64 = 64;
const DEFAULT_SEED: u64 = 0x1933_0514;
const MAX_OPERATION_WAIT_MINUTES: u32 = 1_440;
const MIN_SAMPLES_FOR_VARIATION_CONTRACT: u64 = 3;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HarnessOptions {
    mode: HarnessMode,
    samples: u64,
    seed: u64,
    strategy: Option<Strategy>,
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
    detective: CharacterId,
    opportunity_information: InformationId,
    alternate_opportunity_information: InformationId,
    enterprise: EnterpriseId,
    investigation: Option<crimocracy::core::id::InvestigationId>,
    variation: FixtureVariation,
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
}

#[derive(Default)]
struct Aggregate {
    samples: u64,
    fixture_variations: BTreeSet<FixtureVariation>,
    achieved: u64,
    partial: u64,
    failed: u64,
    aborted: u64,
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
            None => {}
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
            "{label:<6}  samples {:>2}  fixtures {:?}  achieved {:>5.1}%  partial {:>5.1}%  failed {:>5.1}%  aborted {:>5.1}%  standing {:>5.1}%  police {:>5.1}%  cases {:>5.1}%  legal intel {:>5.1}%  police intel {:>5.1}%  case hot {:>5.1}%  case cold {:>5.1}%  case work {}/{}  avg exposure {:>5.1}  avg intel {:>5.1}  avg finish {:>5.0}m  avg property {:>8.0}c -> {:>8.0}c cash @ {:>5.0}m  reports {:>3}  briefs {:>3}  rival attempts {:>3}  departures {:>3}",
            self.samples,
            self.fixture_variations,
            self.percent(self.achieved),
            self.percent(self.partial),
            self.percent(self.failed),
            self.percent(self.aborted),
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
            "[SMOKE] {:<5} terminal {:>4}m; {}; police {}; evidence {}; legal intel {}; police intel {}; follow-up {:?}; case hot {:?}; cold {:?}; intelligence {:?}",
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
    let HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
    } = options;
    debug_assert_eq!(mode, HarnessMode::Full);
    debug_assert!(strategy.is_none());
    let registry = build_registry();

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
    print_metrics(&rush);
    print_metrics(&press);
    print_metrics(&recon);
    print_experience_readout(&rush, &press, &recon);
    validate_branch_financial_isolation(&rush, &press, &recon)?;
    println!(
        "[HARNESS CHECK] Unchanged legitimate and delegated-enterprise systems produced identical cashflow across strategy branches."
    );

    println!("\n--- OPPORTUNITY PORTFOLIO PROBE ---");
    run_opportunity_portfolio_probe(&registry, seed)?;

    println!("\n--- LEGAL FOUNDATION CHECK ---");
    run_legal_foundation_check(&registry)?;

    println!("\n--- NIGHT-TRAP BATCH ({samples} seeds per strategy) ---");
    println!("[BATCH] Running matched seeds for NIGHT TRAP...");
    let (rush_aggregate, press_aggregate, recon_aggregate) =
        run_strategy_batch(&registry, ScenarioProfile::NightTrap, samples, seed)?;
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
        let (rush, press, recon) = run_strategy_batch(&registry, profile, samples, seed)?;
        println!("\n[{}]", profile.label());
        rush.print("RUSH");
        press.print("PRESS");
        recon.print("RECON");
        println!(
            "[BATCH PASS] {} matched-seed checks passed.",
            profile.label()
        );
    }

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
    let mut samples_were_explicit = false;
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
    } else if strategy.is_some() {
        return Err(HarnessCliError::StrategyOnlyInSmoke);
    }
    Ok(Some(HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
    }))
}

fn print_usage() {
    println!(
        "Usage: cargo run --example gameplay_harness -- [--mode smoke|full] [--strategy all|rush|press|recon] [--samples 1..={MAX_BATCH_SAMPLES}] [--seed HEX]"
    );
    println!("  smoke  Fast canonical-path check for the local gate and iteration (default).");
    println!("         --strategy rush|press|recon focuses one branch; default is all.");
    println!("  full   Narrative session, legal check, matched batch, and sensitivity report.");
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
    let same_enterprise_cashflow = rush.enterprise_net_cents == press.enterprise_net_cents
        && press.enterprise_net_cents == recon.enterprise_net_cents;
    if !same_legitimate_cashflow || !same_enterprise_cashflow {
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
            valid_until: Some(SimTime::from_minutes(720)),
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
    } else if narrative && strategy == Strategy::Rush {
        println!(
            "[DECIDE]  Move immediately on the opportunity at 02:10, using only the original street information."
        );
    } else if narrative {
        println!(
            "[DECIDE]  Hit {} at 02:10 and press on through a police response unless leadership later orders otherwise.",
            scenario.variation.target_name(),
        );
    }

    let scheduled_for = match strategy {
        Strategy::Rush | Strategy::Press => SimTime::from_minutes(130),
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
    if scenario.state.now() >= scheduled_for {
        return Err(format!(
            "scenario preparation reached minute {} before burglary schedule {}",
            scenario.state.now().as_minutes(),
            scheduled_for.as_minutes()
        )
        .into());
    }
    let target = scenario.target;
    let title = format!("{} burglary", scenario.variation.target_name());
    let burglary = authorize_burglary(
        &mut scenario,
        strategy,
        target,
        &title,
        scheduled_for,
        burglary_intelligence,
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
                    "[PROCEEDS] Held property estimated at {} cents. This is organizational value, not liquid cash.",
                    proceeds.estimated_value().cents()
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
                "[LIQUIDATE] {} cents estimated property -> {} cents realized resale cash.",
                estimated_value.cents(),
                disposition.realized_value.cents(),
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
        if narrative {
            println!(
                "[DECIDE]  A case is open and the crew's field report is back. Stand down all visible work in {neighborhood_name} until leadership knows whether {police_name} is still developing it."
            );
            println!(
                "[DECIDE]  Watch {police_name} itself at 08:20, outside the known patrol windows, to read whether detectives are still actively working the matter."
            );
        }
        let counterintelligence_title = format!("{police_name} case-heat check");
        let police = scenario.police;
        let counterintelligence = authorize_surveillance_target(
            &mut scenario,
            EntityRef::Organization(police),
            &counterintelligence_title,
            SimTime::from_minutes(500),
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
        if narrative {
            run_until(
                &mut scenario,
                SimTime::from_minutes(2_525),
                narrative,
                &mut metrics,
            )?;
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
        let observation_end = if narrative {
            SimTime::from_minutes(2_880)
        } else {
            SimTime::from_minutes(1_440)
        };
        run_until(&mut scenario, observation_end, narrative, &mut metrics)?;
        let financials = resolve_financial_view(&scenario)?;
        metrics.legitimate_net_cents = Some(financials.legitimate_net_cents);
        metrics.enterprise_net_cents = Some(financials.enterprise_net_cents);
        if narrative {
            print_final_case_audit(&scenario, burglary);
            print_financial_view(&scenario, financials);
            println!("\n[EXECUTIVE BRIEFS]");
            for report in scenario
                .state
                .reports()
                .reports_for(scenario.player)
                .filter(|report| report.kind() == ReportKind::ExecutiveBrief)
            {
                print_report("BRIEF", report, &scenario);
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

    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: variation.neighborhood_name().to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(variation.neighborhood_economy().0),
                    commercial_activity: rating(variation.neighborhood_economy().1),
                    illicit_demand: rating(variation.neighborhood_economy().2),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(variation.neighborhood_police_presence()),
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
            Ok(PatrolWindow::try_new(
                DayMinute::try_new(start)?,
                duration,
                rating(presence),
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

    let enterprise_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::StreetCash,
            label: format!("{} street cash", variation.neighborhood_name()),
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
    .commit(&mut state);
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
    .commit(&mut state);

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
        detective,
        opportunity_information,
        alternate_opportunity_information,
        enterprise,
        investigation: None,
        variation,
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
                (RoleKind::EntrySpecialist, scenario.burglar),
            ]),
            intelligence,
            constraints: vec![
                OperationConstraint::AvoidCasualties,
                OperationConstraint::ProtectLeadershipIdentity,
            ],
            contingencies,
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?)
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
    let selected_operation = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        target,
        &title,
        SimTime::from_minutes(130),
        intelligence,
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
    println!(
        "[PORTFOLIO] Selected {} from player-visible source quality, converted it into {}, and left the weaker opportunity to expire with a lifecycle report.",
        scenario.variation.alternate_target_name(),
        terminal_label(&metrics),
    );
    Ok(())
}

fn run_until_operation_terminal(
    scenario: &mut Scenario,
    operation: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let started_at = scenario.state.now();
    let deadline = started_at + SimDuration::from_minutes(MAX_OPERATION_WAIT_MINUTES);
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
                "[START]   minute {:>4}: {} started.",
                outcome.now.as_minutes(),
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
                    "[CASE COLD] minute {:>4}: {} shelved the case after sustained routine investigation found no actionable subject.",
                    outcome.now.as_minutes(),
                    owner_name
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
                "[EXCEPTION] minute {:>4}: {}",
                outcome.now.as_minutes(),
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
            let follow_up_response = DecisionResponse::Abort;
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
                    "[PLAYER ACTION] minute {:>4}: the crew reported the police response back to Marrow Organization; the organization now knows what the burglar directly experienced.",
                    outcome.now.as_minutes(),
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
                "[AUTONOMY] minute {:>4}: {} independently approached {} using {:?}; pressure {}, margin {}, outcome {:?}.",
                outcome.now.as_minutes(),
                recruiter.name(),
                candidate.name(),
                attempt.approach(),
                attempt.factors().perceived_legal_pressure(),
                attempt.margin(),
                attempt.outcome(),
            );
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
                "[RESULT]  minute {:>4}: {} -> {:?}, exposure {:?}.",
                outcome.now.as_minutes(),
                record.title(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            );
        }
        if !outcome.business_cycles.is_empty() || !outcome.enterprise_cycles.is_empty() {
            println!(
                "[ROUTINE] minute {:>4}: {} legitimate business cycle(s), {} delegated enterprise cycle(s).",
                outcome.now.as_minutes(),
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
            print_report("BRIEF GENERATED", report, scenario);
        }
    }
    validate_harness_state(scenario.registry, &scenario.state)?;
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
        "[WORLD] {} has player-owned {} and {}, target {}, {} jurisdiction, and two rival organizations ({}, {}).",
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
        "\n[FINANCIAL VIEW at minute {}]",
        scenario.state.now().as_minutes()
    );
    println!(
        "  Legitimate front: {} cycle(s), net {} cents.",
        view.legitimate_cycle_count, view.legitimate_net_cents,
    );
    println!(
        "  Delegated gambling: {} cycle(s), net {} cents, street-cash balance {} cents.",
        view.enterprise_cycle_count, view.enterprise_net_cents, view.street_cash_cents,
    );
    println!(
        "  Resale liquidation cash balance: {} cents.",
        view.liquidation_cash_cents,
    );
    println!(
        "  Held operation property: {} operation(s), estimated value {} cents, unliquidated.",
        view.held_property_operations, view.held_property_value_cents,
    );
    println!(
        "  Liquidated operation property: {} disposition(s), realized {} cents.",
        view.liquidated_property_operations, view.liquidated_property_cash_cents,
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

fn print_metrics(metrics: &RunMetrics) {
    let property_acquired = optional_cents(metrics.property_acquired_value_cents);
    let property_realized = optional_cents(metrics.property_realized_cash_cents);
    let liquidation_minute = optional_minute(metrics.liquidation_minute);
    println!(
        "{:<6} [{:<9}]: {}, finish {:?}m, police dispatched {}, police arrived {}, decisions {}, plan items {} {:?}, intel {:?}, exposure {:?}/{:?}, property {} -> {} cash at {}, case {}, evidence {}, player legal intel {}, police intel {}, follow-up {:?}/{} info (case hot {:?}), cold confirmed {:?} @ {:?}, case work {}/{}, surveillance discoveries {}, reports {}, briefs {}, autonomous recruitment {}, player departures {}",
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
        "pressure changes personnel relationships without a scripted player event",
    );
    print_loop_checkpoint(
        "routine",
        rush.legitimate_net_cents == press.legitimate_net_cents
            && press.legitimate_net_cents == recon.legitimate_net_cents
            && rush.enterprise_net_cents == press.enterprise_net_cents
            && press.enterprise_net_cents == recon.enterprise_net_cents,
        "delegated legitimate and illicit enterprises continue while leadership focuses on exceptions",
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
        "  - Consequence leverage: PRESS exposed {} evidence item(s), {} legal-activity information item(s), read the case as still hot at minute ~500, then confirmed it shelved at minute {}; RECON realized {} cents of resale cash.",
        press.evidence_count,
        press.player_legal_activity_information,
        press.case_cold_minute.unwrap_or_default(),
        recon.property_realized_cash_cents.unwrap_or_default(),
    );
    println!(
        "  - Time tradeoff: RECON finished at minute {} versus RUSH at minute {}; the extra planning time bought lower exposure and liquid value in this matched fixture.",
        recon.burglary_terminal_minute.unwrap_or_default(),
        rush.burglary_terminal_minute.unwrap_or_default(),
    );
    println!("Current experience gaps exposed by this fixture:");
    println!(
        "  - The consequence arc now closes: an open case can be read, outlasted by standing down, and verified shelved. Disrupting evidence, influencing counsel, or changing a prosecution outcome are still not modeled."
    );
    println!(
        "  - The portfolio probe now covers prioritization and expiry across competing opportunities, but it still uses one operation type and does not test resource contention between simultaneous crews."
    );
    println!(
        "  - The fixed RUSH/PRESS/RECON policies are calibration treatments; they expose causal differences but are not evidence that an actual player would choose the same policies."
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

fn choose_safe_start_from_patrol_report(
    now: SimTime,
    report: &str,
    operation_duration: SimDuration,
    uncertainty_buffer: SimDuration,
) -> Result<SimTime, Box<dyn Error>> {
    let windows = parse_patrol_windows(report);
    if windows.is_empty() {
        return Err(format!(
            "surveillance report did not contain actionable recurring patrol windows: {report}"
        )
        .into());
    }
    let duration = u64::from(operation_duration.as_minutes());
    let buffer = u64::from(uncertainty_buffer.as_minutes());
    let earliest = now.as_minutes().saturating_add(1);
    let first_candidate = earliest.div_ceil(30).saturating_mul(30);
    for candidate in (first_candidate..first_candidate.saturating_add(2_880)).step_by(30) {
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
    Err("no safe operation window was derivable from the surveillance report".into())
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

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value)
        .expect("gameplay harness relationship levels are authored within 0..=100")
}

#[cfg(test)]
mod tests {
    use super::{
        choose_safe_start_from_patrol_report, parse_options, parse_patrol_windows,
        run_opportunity_portfolio_probe, run_smoke, FixtureVariation, HarnessCliError, HarnessMode,
        HarnessOptions, ScenarioProfile, Strategy, DEFAULT_SEED,
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
        )
        .expect_err("the harness must not infer a safe time from vague surveillance");

        assert!(error
            .to_string()
            .contains("did not contain actionable recurring patrol windows"));
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
    #[ignore = "controlled smoke contract runs in its focused local gate lane"]
    fn smoke_mode_covers_canonical_paths() {
        run_smoke(DEFAULT_SEED, None)
            .expect("smoke harness should pass its canonical-path contract");
    }
}
