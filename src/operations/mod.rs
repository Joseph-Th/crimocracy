//! Semantic operation plans, execution state, and outcomes; sibling systems own authorization and resolution.

pub(crate) mod operation_abort;
pub(crate) mod operation_economics;
pub(crate) mod operation_execution;
pub(crate) mod operation_intelligence;
pub(crate) mod operation_objective;
pub(crate) mod operation_state;
pub mod operation_system;
pub(crate) mod police_response_integration;
pub mod property_disposition;
pub(crate) mod surveillance_integration;

pub use operation_state::OperationState;

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, BusinessId, CharacterId, DecisionRequestId, EvidenceId, FinancialAccountId,
    HistoryEventId, InformationId, InvestigationId, LedgerTransactionId, NeighborhoodId,
    OperationId, OrganizationId, PoliceResponseId, ReportId,
};
use crate::core::time::SimTime;
use crate::finance::Money;
use crate::intelligence::{InformationSignal, InformationTopic};
use crate::world::Rating;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationKind {
    Burglary,
    Robbery,
    Hijacking,
    Smuggling,
    Intimidation,
    Surveillance,
    WitnessPressure,
    DocumentTheft,
    GamblingEvent,
    Extraction,
    Sabotage,
    Arson,
}

impl OperationKind {
    /// One canonical objective family per operation kind. Target shape and current practical
    /// actionability are validated separately, but consumers such as opportunity discovery must
    /// not maintain their own parallel kind-to-objective table.
    pub(crate) const fn objective_kind(self) -> OperationObjectiveKind {
        match self {
            Self::Burglary | Self::Hijacking | Self::DocumentTheft => {
                OperationObjectiveKind::AcquireProperty
            }
            Self::Robbery | Self::Smuggling | Self::Intimidation | Self::GamblingEvent => {
                OperationObjectiveKind::ObtainCash
            }
            Self::Surveillance => OperationObjectiveKind::GatherInformation,
            Self::WitnessPressure => OperationObjectiveKind::Frighten,
            Self::Extraction => OperationObjectiveKind::FreeDetainee,
            Self::Sabotage | Self::Arson => OperationObjectiveKind::DisruptBusiness,
        }
    }

    /// Builds this kind's sole objective family around a candidate entity. Extraction is the one
    /// objective whose target is stored as a typed character id, so a non-character cannot even
    /// form a candidate objective. Shape validation remains centralized in
    /// `is_valid_operation_objective`.
    pub(crate) fn objective_for_target(self, target: EntityRef) -> Option<OperationObjective> {
        Some(match self.objective_kind() {
            OperationObjectiveKind::AcquireProperty => {
                OperationObjective::AcquireProperty { target }
            }
            OperationObjectiveKind::ObtainCash => OperationObjective::ObtainCash { target },
            OperationObjectiveKind::Frighten => OperationObjective::Frighten { target },
            OperationObjectiveKind::GatherInformation => {
                OperationObjective::GatherInformation { target }
            }
            OperationObjectiveKind::FreeDetainee => {
                let EntityRef::Character(target) = target else {
                    return None;
                };
                OperationObjective::FreeDetainee { target }
            }
            OperationObjectiveKind::DisruptBusiness => {
                OperationObjective::DisruptBusiness { target }
            }
        })
    }

    /// Whether this kind has an authored property-proceeds effect and therefore may
    /// authorize a property-acquisition objective.
    pub(crate) const fn can_acquire_property(self) -> bool {
        matches!(
            self.objective_kind(),
            OperationObjectiveKind::AcquireProperty
        )
    }

    /// Whether this kind has an authored cash-proceeds effect and therefore may
    /// authorize a cash-acquisition objective.
    pub(crate) const fn can_take_cash(self) -> bool {
        matches!(self.objective_kind(), OperationObjectiveKind::ObtainCash)
    }

    /// Intrinsic ownership relationship between the sponsor and a business objective target.
    /// This is operation semantics, not balance tuning: gambling is a hosted house event, while
    /// take/disruption operations act against a counterparty's property.
    pub(crate) const fn business_target_ownership(
        self,
    ) -> Option<OperationBusinessTargetOwnership> {
        match self {
            Self::GamblingEvent => Some(OperationBusinessTargetOwnership::SponsorOwned),
            Self::Burglary
            | Self::Robbery
            | Self::Hijacking
            | Self::Smuggling
            | Self::Intimidation
            | Self::DocumentTheft
            | Self::Sabotage
            | Self::Arson => Some(OperationBusinessTargetOwnership::Foreign),
            Self::Surveillance | Self::WitnessPressure | Self::Extraction => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationBusinessTargetOwnership {
    Foreign,
    SponsorOwned,
}

pub const ALL_OPERATION_KINDS: [OperationKind; 12] = [
    OperationKind::Burglary,
    OperationKind::Robbery,
    OperationKind::Hijacking,
    OperationKind::Smuggling,
    OperationKind::Intimidation,
    OperationKind::Surveillance,
    OperationKind::WitnessPressure,
    OperationKind::DocumentTheft,
    OperationKind::GamblingEvent,
    OperationKind::Extraction,
    OperationKind::Sabotage,
    OperationKind::Arson,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationApproach {
    Covert,
    Deceptive,
    Intimidating,
    Violent,
    InsideAssistance,
    Opportunistic,
}

pub const ALL_OPERATION_APPROACHES: [OperationApproach; 6] = [
    OperationApproach::Covert,
    OperationApproach::Deceptive,
    OperationApproach::Intimidating,
    OperationApproach::Violent,
    OperationApproach::InsideAssistance,
    OperationApproach::Opportunistic,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoleKind {
    Driver,
    Lookout,
    EntrySpecialist,
    SafeSpecialist,
    Muscle,
    InsideContact,
    Coordinator,
    Surveillance,
    Negotiator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationObjective {
    AcquireProperty {
        target: EntityRef,
    },
    ObtainCash {
        target: EntityRef,
    },
    Frighten {
        target: EntityRef,
    },
    GatherInformation {
        target: EntityRef,
    },
    /// Extract any detained character from custody — an organization member, or a third
    /// party whose release serves the sponsor. Success releases the active arrest through
    /// the canonical arrest-release path.
    FreeDetainee {
        target: CharacterId,
    },
    /// Damage a business's operating capacity. Success disrupts the target's economy
    /// through the canonical business-economy disruption path for an authored duration.
    DisruptBusiness {
        target: EntityRef,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationObjectiveKind {
    AcquireProperty,
    ObtainCash,
    Frighten,
    GatherInformation,
    FreeDetainee,
    DisruptBusiness,
}

impl OperationObjective {
    pub fn kind(&self) -> OperationObjectiveKind {
        match self {
            Self::AcquireProperty { .. } => OperationObjectiveKind::AcquireProperty,
            Self::ObtainCash { .. } => OperationObjectiveKind::ObtainCash,
            Self::Frighten { .. } => OperationObjectiveKind::Frighten,
            Self::GatherInformation { .. } => OperationObjectiveKind::GatherInformation,
            Self::FreeDetainee { .. } => OperationObjectiveKind::FreeDetainee,
            Self::DisruptBusiness { .. } => OperationObjectiveKind::DisruptBusiness,
        }
    }

    pub(crate) fn referenced_entities(&self) -> Vec<EntityRef> {
        match self {
            Self::AcquireProperty { target }
            | Self::ObtainCash { target }
            | Self::Frighten { target }
            | Self::GatherInformation { target }
            | Self::DisruptBusiness { target } => vec![*target],
            Self::FreeDetainee { target } => vec![EntityRef::Character(*target)],
        }
    }

    /// The business whose gross potential sizes a take-like objective, if any. Theft and
    /// collection kinds use it as the depleted counterparty; gambling uses it as the venue whose
    /// customer throughput sizes the house take. The economics owner decides replenishment
    /// independently per operation kind.
    pub(crate) fn taken_business(&self) -> Option<BusinessId> {
        let target = match self {
            Self::AcquireProperty { target } | Self::ObtainCash { target } => target,
            Self::Frighten { .. }
            | Self::GatherInformation { .. }
            | Self::FreeDetainee { .. }
            | Self::DisruptBusiness { .. } => return None,
        };
        match target {
            EntityRef::Business(business) => Some(*business),
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Neighborhood(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => None,
        }
    }
}

/// Authorable execution boundaries backed by current operation mechanics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationConstraint {
    /// Inclusive completion deadline. The operation may resolve on this minute but not after it.
    CompleteBy(SimTime),
    /// Authorization gate: the plan must carry organization-held intelligence of the given
    /// topic that is relevant to the objective and still has nonzero planning value at the
    /// earliest planned start. Reconnaissance is therefore a usable prerequisite, not flavor.
    RequireIntelligenceTopic(InformationTopic),
}

/// Standing reactions backed by police-response and leadership-follow-up mechanics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationContingency {
    AbortOnPoliceArrivalBeforeEntry,
    RequestDecisionOnPoliceArrival,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationStatus {
    Authorized,
    InProgress,
    AwaitingDecision,
    Completed,
    Aborted,
}

/// The live assignment statuses whose participants are booked: a character holding any of
/// these cannot take overlapping work. Custody preempts live bookings through the canonical
/// detention-abort path. Terminal operations release their participants and are excluded from
/// every booking scan.
pub const ACTIVE_ASSIGNMENT_STATUSES: [OperationStatus; 3] = [
    OperationStatus::Authorized,
    OperationStatus::InProgress,
    OperationStatus::AwaitingDecision,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationAbortPhase {
    BeforeStart,
    InProgress,
    AwaitingDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationAbortCause {
    AuthorityOrder,
    Decision(DecisionRequestId),
    PoliceArrival(PoliceResponseId),
    DeadlineMissed,
    ParticipantDetained(CharacterId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAbortArtifacts {
    information: InformationId,
    report: ReportId,
    history_event: HistoryEventId,
    /// District-scoped police-response knowledge the organization holds after a pre-entry
    /// police-arrival abort: the crew was debriefed through production paths, so leadership
    /// knows how the responding authority moved in that neighborhood.
    police_activity_information: Option<InformationId>,
}

impl OperationAbortArtifacts {
    pub fn information(self) -> InformationId {
        self.information
    }

    pub fn report(self) -> ReportId {
        self.report
    }

    pub fn history_event(self) -> HistoryEventId {
        self.history_event
    }

    pub fn police_activity_information(self) -> Option<InformationId> {
        self.police_activity_information
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAbortRecord {
    aborted_at: SimTime,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    artifacts: Option<OperationAbortArtifacts>,
}

impl OperationAbortRecord {
    pub fn aborted_at(self) -> SimTime {
        self.aborted_at
    }

    pub fn phase(self) -> OperationAbortPhase {
        self.phase
    }

    pub fn cause(self) -> OperationAbortCause {
        self.cause
    }

    pub fn artifacts(self) -> Option<OperationAbortArtifacts> {
        self.artifacts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationObjectiveOutcome {
    Achieved,
    Partial,
    Failed,
}

/// A practical condition that made a tactically viable objective impossible at the instant the
/// operation resolved. These are persisted causal facts because several source domains retain
/// only current state, so later restore validation cannot safely reconstruct the historical
/// reason from whatever the target looks like now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationObjectiveBlocker {
    TargetBusinessOwnershipMismatch,
    TargetEconomyInactive,
    NoPressureableWitnessCase,
    ExtractionCustodyEnded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationExposureLevel {
    None,
    Trace,
    Witnessed,
    Identifying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationExposureFactors {
    stealth_average: Rating,
    target_police_presence: Option<Rating>,
    police_response_arrived: bool,
    approach_adjustment: i8,
    intelligence_mitigation: u8,
    variance: i8,
}

impl OperationExposureFactors {
    pub fn stealth_average(self) -> Rating {
        self.stealth_average
    }

    pub fn target_police_presence(self) -> Option<Rating> {
        self.target_police_presence
    }

    pub fn police_response_arrived(self) -> bool {
        self.police_response_arrived
    }

    pub fn approach_adjustment(self) -> i8 {
        self.approach_adjustment
    }

    pub fn intelligence_mitigation(self) -> u8 {
        self.intelligence_mitigation
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationExposureRecord {
    level: OperationExposureLevel,
    score: i16,
    factors: OperationExposureFactors,
    neighborhood: Option<NeighborhoodId>,
    identified_character: Option<CharacterId>,
    investigation: Option<InvestigationId>,
    evidence: BTreeSet<EvidenceId>,
}

impl OperationExposureRecord {
    pub fn level(&self) -> OperationExposureLevel {
        self.level
    }

    pub fn score(&self) -> i16 {
        self.score
    }

    pub fn factors(&self) -> OperationExposureFactors {
        self.factors
    }

    pub fn neighborhood(&self) -> Option<NeighborhoodId> {
        self.neighborhood
    }

    pub fn identified_character(&self) -> Option<CharacterId> {
        self.identified_character
    }

    pub fn investigation(&self) -> Option<InvestigationId> {
        self.investigation
    }

    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPropertyProceedsRecord {
    target: EntityRef,
    estimated_value: Money,
}

impl OperationPropertyProceedsRecord {
    pub(crate) fn new(target: EntityRef, estimated_value: Money) -> Self {
        Self {
            target,
            estimated_value,
        }
    }

    pub fn target(self) -> EntityRef {
        self.target
    }

    pub fn estimated_value(self) -> Money {
        self.estimated_value
    }
}

/// Cash proceeds from a completed operation. Depending on operation kind this may be stolen cash,
/// a protection collection, delivery payment, or gambling house take. Unlike held property, cash
/// needs no resale venue; it is deposited through the canonical cash-disposition command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationCashProceedsRecord {
    target: EntityRef,
    amount: Money,
}

impl OperationCashProceedsRecord {
    pub(crate) fn new(target: EntityRef, amount: Money) -> Self {
        Self { target, amount }
    }

    pub fn target(self) -> EntityRef {
        self.target
    }

    pub fn amount(self) -> Money {
        self.amount
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationCashDispositionRecord {
    disposed_at: SimTime,
    realized_value: Money,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    transaction: LedgerTransactionId,
    information: InformationId,
    report: ReportId,
}

impl OperationCashDispositionRecord {
    pub fn disposed_at(self) -> SimTime {
        self.disposed_at
    }

    pub fn realized_value(self) -> Money {
        self.realized_value
    }

    pub fn cash_account(self) -> FinancialAccountId {
        self.cash_account
    }

    pub fn settlement_account(self) -> FinancialAccountId {
        self.settlement_account
    }

    pub fn transaction(self) -> LedgerTransactionId {
        self.transaction
    }

    pub fn information(self) -> InformationId {
        self.information
    }

    pub fn report(self) -> ReportId {
        self.report
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPropertyDispositionRecord {
    disposed_at: SimTime,
    venue: BusinessId,
    venue_version: u32,
    realized_value: Money,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    transaction: LedgerTransactionId,
    information: InformationId,
    report: ReportId,
}

impl OperationPropertyDispositionRecord {
    pub fn disposed_at(self) -> SimTime {
        self.disposed_at
    }

    pub fn venue(self) -> BusinessId {
        self.venue
    }

    pub fn venue_version(self) -> u32 {
        self.venue_version
    }

    pub fn realized_value(self) -> Money {
        self.realized_value
    }

    pub fn cash_account(self) -> FinancialAccountId {
        self.cash_account
    }

    pub fn settlement_account(self) -> FinancialAccountId {
        self.settlement_account
    }

    pub fn transaction(self) -> LedgerTransactionId {
        self.transaction
    }

    pub fn information(self) -> InformationId {
        self.information
    }

    pub fn report(self) -> ReportId {
        self.report
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResolutionFactors {
    role_capability_average: Rating,
    leader_capability: Option<Rating>,
    intelligence_quality: Rating,
    intelligence_adjustment: i8,
    intelligence_topics_covered: u8,
    intelligence_topics_relevant: u8,
    target_police_presence: Option<Rating>,
    police_response_arrived: bool,
    approach_adjustment: i8,
    time_pressure: u8,
    variance: i8,
}

impl OperationResolutionFactors {
    pub fn role_capability_average(self) -> Rating {
        self.role_capability_average
    }

    pub fn leader_capability(self) -> Option<Rating> {
        self.leader_capability
    }

    pub fn intelligence_quality(self) -> Rating {
        self.intelligence_quality
    }

    pub fn intelligence_adjustment(self) -> i8 {
        self.intelligence_adjustment
    }

    pub fn intelligence_topics_covered(self) -> u8 {
        self.intelligence_topics_covered
    }

    pub fn intelligence_topics_relevant(self) -> u8 {
        self.intelligence_topics_relevant
    }

    pub fn target_police_presence(self) -> Option<Rating> {
        self.target_police_presence
    }

    pub fn police_response_arrived(self) -> bool {
        self.police_response_arrived
    }

    pub fn approach_adjustment(self) -> i8 {
        self.approach_adjustment
    }

    pub fn time_pressure(self) -> u8 {
        self.time_pressure
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationResolutionRecord {
    resolved_at: SimTime,
    objective_outcome: OperationObjectiveOutcome,
    objective_blocker: Option<OperationObjectiveBlocker>,
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposureRecord,
    property_proceeds: Option<OperationPropertyProceedsRecord>,
    cash_proceeds: Option<OperationCashProceedsRecord>,
    /// Custody relationship observed when an extraction resolved. This snapshot is persisted
    /// because custody may end in the same minute or later, so later validation cannot infer
    /// whether the target was actually detained when the crew reached the objective.
    extraction_arrest: Option<ArrestId>,
    discovered_information: BTreeSet<InformationId>,
    /// Topic/subject/semantic triples actually produced by a surveillance resolution. Persisted
    /// because sightline conditions and the exact typed facts observed at that minute are not
    /// re-derivable after later state changes.
    surveillance_signatures: BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)>,
    legal_activity_information: Option<InformationId>,
    after_action_information: InformationId,
    after_action_report: ReportId,
    history_event: HistoryEventId,
}

impl OperationResolutionRecord {
    pub fn resolved_at(&self) -> SimTime {
        self.resolved_at
    }

    pub fn objective_outcome(&self) -> OperationObjectiveOutcome {
        self.objective_outcome
    }

    pub fn objective_blocker(&self) -> Option<OperationObjectiveBlocker> {
        self.objective_blocker
    }

    pub fn execution_margin(&self) -> i16 {
        self.execution_margin
    }

    pub fn factors(&self) -> OperationResolutionFactors {
        self.factors
    }

    pub fn exposure(&self) -> &OperationExposureRecord {
        &self.exposure
    }

    pub fn property_proceeds(&self) -> Option<OperationPropertyProceedsRecord> {
        self.property_proceeds
    }

    pub fn cash_proceeds(&self) -> Option<OperationCashProceedsRecord> {
        self.cash_proceeds
    }

    pub fn extraction_arrest(&self) -> Option<ArrestId> {
        self.extraction_arrest
    }

    pub fn discovered_information(&self) -> &BTreeSet<InformationId> {
        &self.discovered_information
    }

    pub fn surveillance_signatures(
        &self,
    ) -> &BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)> {
        &self.surveillance_signatures
    }

    pub fn legal_activity_information(&self) -> Option<InformationId> {
        self.legal_activity_information
    }

    pub fn after_action_information(&self) -> InformationId {
        self.after_action_information
    }

    pub fn after_action_report(&self) -> ReportId {
        self.after_action_report
    }

    pub fn history_event(&self) -> HistoryEventId {
        self.history_event
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationIdentity {
    id: OperationId,
    title: String,
    kind: OperationKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationCommand {
    responsible_organization: OrganizationId,
    leader: CharacterId,
    objective: OperationObjective,
    /// Exact custody relationship an extraction was authorized against. A later re-arrest is a
    /// different legal event and must not silently become the target of an already-planned job.
    extraction_arrest: Option<ArrestId>,
    approach: OperationApproach,
    roles: BTreeMap<RoleKind, CharacterId>,
    intelligence: BTreeSet<InformationId>,
    constraints: Vec<OperationConstraint>,
    contingencies: Vec<OperationContingency>,
    authorized_at: SimTime,
    scheduled_for: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationRuntime {
    status: OperationStatus,
    started_at: Option<SimTime>,
    resolution_due_at: Option<SimTime>,
    entry_at: Option<SimTime>,
    police_response: Option<PoliceResponseId>,
    awaiting_decision_since: Option<SimTime>,
    resolution: Option<OperationResolutionRecord>,
    property_disposition: Option<OperationPropertyDispositionRecord>,
    cash_disposition: Option<OperationCashDispositionRecord>,
    abort: Option<OperationAbortRecord>,
    version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationRecord {
    identity: OperationIdentity,
    command: OperationCommand,
    runtime: OperationRuntime,
}

impl OperationRecord {
    pub fn id(&self) -> OperationId {
        self.identity.id
    }

    pub fn title(&self) -> &str {
        &self.identity.title
    }

    pub fn kind(&self) -> OperationKind {
        self.identity.kind
    }

    pub fn responsible_organization(&self) -> OrganizationId {
        self.command.responsible_organization
    }

    pub fn leader(&self) -> CharacterId {
        self.command.leader
    }

    pub fn objective(&self) -> &OperationObjective {
        &self.command.objective
    }

    pub fn extraction_arrest(&self) -> Option<ArrestId> {
        self.command.extraction_arrest
    }

    pub fn approach(&self) -> OperationApproach {
        self.command.approach
    }

    pub fn roles(&self) -> &BTreeMap<RoleKind, CharacterId> {
        &self.command.roles
    }

    /// Every participant bound to the record — leader plus all role holders. Participant
    /// release and double-booking checks depend on this union being defined exactly once.
    pub fn participants(&self) -> BTreeSet<CharacterId> {
        let mut participants = BTreeSet::from([self.command.leader]);
        participants.extend(self.command.roles.values().copied());
        participants
    }

    pub fn intelligence(&self) -> &BTreeSet<InformationId> {
        &self.command.intelligence
    }

    pub fn constraints(&self) -> &[OperationConstraint] {
        &self.command.constraints
    }

    pub fn contingencies(&self) -> &[OperationContingency] {
        &self.command.contingencies
    }

    pub fn authorized_at(&self) -> SimTime {
        self.command.authorized_at
    }

    pub fn scheduled_for(&self) -> SimTime {
        self.command.scheduled_for
    }

    pub fn status(&self) -> OperationStatus {
        self.runtime.status
    }

    pub fn started_at(&self) -> Option<SimTime> {
        self.runtime.started_at
    }

    pub fn resolution_due_at(&self) -> Option<SimTime> {
        self.runtime.resolution_due_at
    }

    pub fn entry_at(&self) -> Option<SimTime> {
        self.runtime.entry_at
    }

    pub fn police_response(&self) -> Option<PoliceResponseId> {
        self.runtime.police_response
    }

    pub fn awaiting_decision_since(&self) -> Option<SimTime> {
        self.runtime.awaiting_decision_since
    }

    pub fn resolution(&self) -> Option<&OperationResolutionRecord> {
        self.runtime.resolution.as_ref()
    }

    pub fn property_disposition(&self) -> Option<OperationPropertyDispositionRecord> {
        self.runtime.property_disposition
    }

    pub fn cash_disposition(&self) -> Option<OperationCashDispositionRecord> {
        self.runtime.cash_disposition
    }

    pub fn abort_record(&self) -> Option<OperationAbortRecord> {
        self.runtime.abort
    }

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug)]
pub struct OperationDraft {
    pub title: String,
    pub kind: OperationKind,
    pub responsible_organization: OrganizationId,
    pub leader: CharacterId,
    pub objective: OperationObjective,
    pub approach: OperationApproach,
    pub roles: BTreeMap<RoleKind, CharacterId>,
    pub intelligence: BTreeSet<InformationId>,
    pub constraints: Vec<OperationConstraint>,
    pub contingencies: Vec<OperationContingency>,
    pub scheduled_for: SimTime,
}
