//! Serializable application state; subsystem state is owned here and mutated through systems.
//!
//! `AppState` is the aggregate root for domain state, deterministic RNG streams, and runtime metadata.
//! Find the owning field below, then the canonical `validate_* → commit` in that
//! owner's `*_system.rs` (see the `ARCHITECTURE.md` source map).
//! All `//!` headers name the owner; `core::simulation::run_tick` orchestrates the ordered domain passes.

// Current schema — bump on any AppState layout change; `STATUS.md` must stay in sync.

use crate::contacts::ContactState;
use crate::core::attention::{AttentionClass, AttentionSettings};
use crate::core::id::{IdCounters, OrganizationId};
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::DecisionState;
use crate::delegation::DelegationState;
use crate::economy::EconomyState;
use crate::enterprises::EnterpriseState;
use crate::finance::FinanceState;
use crate::history::HistoryState;
use crate::intelligence::IntelligenceState;
use crate::legal::LegalState;
use crate::operations::OperationState;
use crate::opportunities::OpportunityState;
use crate::recruitment::RecruitmentState;
use crate::reports::ReportState;
use crate::reputation::ReputationState;
use crate::social::SocialState;
use crate::world::WorldState;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

pub const CURRENT_STATE_SCHEMA_VERSION: u16 = 91;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StateMetadata {
    schema_version: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SimulationRuntime {
    now: SimTime,
    operation_rng: ChaCha8Rng,
    investigation_rng: ChaCha8Rng,
    business_rng: ChaCha8Rng,
    enterprise_rng: ChaCha8Rng,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CampaignRuntime {
    player_organization: Option<OrganizationId>,
    attention: AttentionSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppState {
    metadata: StateMetadata,
    simulation: SimulationRuntime,
    campaign: CampaignRuntime,
    pub(crate) ids: IdCounters,
    pub(crate) world: WorldState,
    pub(crate) contacts: ContactState,
    pub(crate) decisions: DecisionState,
    pub(crate) delegation: DelegationState,
    pub(crate) economy: EconomyState,
    pub(crate) enterprises: EnterpriseState,
    pub(crate) finance: FinanceState,
    pub(crate) social: SocialState,
    pub(crate) intelligence: IntelligenceState,
    pub(crate) operations: OperationState,
    pub(crate) opportunities: OpportunityState,
    pub(crate) recruitment: RecruitmentState,
    pub(crate) reputation: ReputationState,
    pub(crate) legal: LegalState,
    pub(crate) reports: ReportState,
    pub(crate) history: HistoryState,
}

impl AppState {
    pub fn new(seed: u64) -> Self {
        Self {
            metadata: StateMetadata {
                schema_version: CURRENT_STATE_SCHEMA_VERSION,
            },
            simulation: SimulationRuntime {
                now: SimTime::ZERO,
                operation_rng: ChaCha8Rng::seed_from_u64(domain_seed(seed, 0x4F50_4552)),
                investigation_rng: ChaCha8Rng::seed_from_u64(domain_seed(seed, 0x494E_5653)),
                business_rng: ChaCha8Rng::seed_from_u64(domain_seed(seed, 0x4255_5349)),
                enterprise_rng: ChaCha8Rng::seed_from_u64(domain_seed(seed, 0x454E_5452)),
            },
            campaign: CampaignRuntime::default(),
            ids: IdCounters::new(),
            world: WorldState::new(),
            contacts: ContactState::new(),
            decisions: DecisionState::new(),
            delegation: DelegationState::new(),
            economy: EconomyState::new(),
            enterprises: EnterpriseState::new(),
            finance: FinanceState::new(),
            social: SocialState::new(),
            intelligence: IntelligenceState::new(),
            operations: OperationState::new(),
            opportunities: OpportunityState::new(),
            recruitment: RecruitmentState::new(),
            reputation: ReputationState::new(),
            legal: LegalState::new(),
            reports: ReportState::new(),
            history: HistoryState::new(),
        }
    }

    pub fn now(&self) -> SimTime {
        self.simulation.now
    }

    pub fn player_organization(&self) -> Option<OrganizationId> {
        self.campaign.player_organization
    }

    pub fn world(&self) -> &WorldState {
        &self.world
    }

    pub fn contacts(&self) -> &ContactState {
        &self.contacts
    }

    pub fn decisions(&self) -> &DecisionState {
        &self.decisions
    }

    pub fn delegation(&self) -> &DelegationState {
        &self.delegation
    }

    pub fn economy(&self) -> &EconomyState {
        &self.economy
    }

    pub fn enterprises(&self) -> &EnterpriseState {
        &self.enterprises
    }

    pub fn finance(&self) -> &FinanceState {
        &self.finance
    }

    pub fn social(&self) -> &SocialState {
        &self.social
    }

    pub fn intelligence(&self) -> &IntelligenceState {
        &self.intelligence
    }

    pub fn operations(&self) -> &OperationState {
        &self.operations
    }

    pub fn opportunities(&self) -> &OpportunityState {
        &self.opportunities
    }

    pub fn recruitment(&self) -> &RecruitmentState {
        &self.recruitment
    }

    pub fn reputation(&self) -> &ReputationState {
        &self.reputation
    }

    pub fn legal(&self) -> &LegalState {
        &self.legal
    }

    pub fn reports(&self) -> &ReportState {
        &self.reports
    }

    pub fn history(&self) -> &HistoryState {
        &self.history
    }

    pub fn attention_settings(&self) -> &AttentionSettings {
        &self.campaign.attention
    }

    /// Canonical mutation path for the persistent auto-pause preference. Only interrupting
    /// classes are settable: decisions carry `Exception` or `Crisis` attention exclusively,
    /// so a stored preference for any other class could never be observed.
    pub fn set_auto_pause(
        &mut self,
        attention: AttentionClass,
        enabled: bool,
    ) -> Result<(), crate::core::attention::AttentionSettingsError> {
        if !matches!(
            attention,
            AttentionClass::Exception | AttentionClass::Crisis
        ) {
            return Err(crate::core::attention::AttentionSettingsError::UnsupportedAutoPauseClass);
        }
        if enabled {
            self.campaign.attention.auto_pause.insert(attention);
        } else {
            self.campaign.attention.auto_pause.remove(&attention);
        }
        Ok(())
    }

    pub(crate) fn state_schema_version(&self) -> u16 {
        self.metadata.schema_version
    }

    /// Reconstructs every non-authoritative lookup/scheduling projection from persisted records.
    /// Save serialization skips these indexes entirely, so restore must run this before normal
    /// invariant validation or any indexed read.
    pub(crate) fn rebuild_derived_indexes_after_restore(&mut self) -> bool {
        self.world.rebuild_derived_indexes();
        self.contacts.rebuild_derived_indexes();
        self.decisions.rebuild_derived_indexes();
        self.delegation.rebuild_derived_indexes();
        self.economy.rebuild_derived_indexes();
        self.enterprises.rebuild_derived_indexes();
        if !self.finance.rebuild_derived_indexes() {
            return false;
        }
        self.social.rebuild_derived_indexes();
        self.intelligence.rebuild_derived_indexes();
        self.operations.rebuild_derived_indexes();
        self.opportunities.rebuild_derived_indexes();
        self.recruitment.rebuild_derived_indexes();
        self.legal.rebuild_derived_indexes();
        self.reports.rebuild_derived_indexes();
        true
    }

    pub(crate) fn set_player_organization(&mut self, organization: OrganizationId) {
        self.campaign.player_organization = Some(organization);
    }

    pub(crate) fn advance_clock(&mut self, duration: SimDuration) {
        self.simulation.now = self.simulation.now + duration;
    }

    #[cfg(test)]
    pub(crate) fn set_now_for_test(&mut self, now: SimTime) {
        self.simulation.now = now;
    }

    pub(crate) fn operation_rng_mut(&mut self) -> &mut ChaCha8Rng {
        &mut self.simulation.operation_rng
    }

    pub(crate) fn investigation_rng_mut(&mut self) -> &mut ChaCha8Rng {
        &mut self.simulation.investigation_rng
    }

    pub(crate) fn business_rng_mut(&mut self) -> &mut ChaCha8Rng {
        &mut self.simulation.business_rng
    }

    pub(crate) fn enterprise_rng_mut(&mut self) -> &mut ChaCha8Rng {
        &mut self.simulation.enterprise_rng
    }
}

fn domain_seed(seed: u64, domain: u64) -> u64 {
    let mut value = seed ^ domain.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests;
