//! Serializable application state; subsystem state is owned here and mutated through systems.

use crate::core::attention::{AttentionClass, AttentionSettings};
use crate::core::id::{IdCounters, OrganizationId};
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::DecisionState;
use crate::delegation::DelegationState;
use crate::finance::FinanceState;
use crate::history::HistoryState;
use crate::intelligence::IntelligenceState;
use crate::legal::LegalState;
use crate::operations::OperationState;
use crate::reports::ReportState;
use crate::social::SocialState;
use crate::world::WorldState;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

pub const CURRENT_STATE_SCHEMA_VERSION: u16 = 6;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StateMetadata {
    schema_version: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SimulationRuntime {
    now: SimTime,
    rng: ChaCha8Rng,
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
    pub(crate) decisions: DecisionState,
    pub(crate) delegation: DelegationState,
    pub(crate) finance: FinanceState,
    pub(crate) social: SocialState,
    pub(crate) intelligence: IntelligenceState,
    pub(crate) operations: OperationState,
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
                rng: ChaCha8Rng::seed_from_u64(seed),
            },
            campaign: CampaignRuntime::default(),
            ids: IdCounters::new(),
            world: WorldState::new(),
            decisions: DecisionState::new(),
            delegation: DelegationState::new(),
            finance: FinanceState::new(),
            social: SocialState::new(),
            intelligence: IntelligenceState::new(),
            operations: OperationState::new(),
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

    pub fn decisions(&self) -> &DecisionState {
        &self.decisions
    }

    pub fn delegation(&self) -> &DelegationState {
        &self.delegation
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

    pub(crate) fn state_schema_version(&self) -> u16 {
        self.metadata.schema_version
    }

    pub(crate) fn set_player_organization(&mut self, organization: OrganizationId) {
        self.campaign.player_organization = Some(organization);
    }

    pub(crate) fn set_attention_auto_pause(&mut self, attention: AttentionClass, enabled: bool) {
        if enabled {
            self.campaign.attention.auto_pause.insert(attention);
        } else {
            self.campaign.attention.auto_pause.remove(&attention);
        }
    }

    pub(crate) fn advance_clock(&mut self, duration: SimDuration) {
        self.simulation.now = self.simulation.now + duration;
    }

    pub(crate) fn rng_mut(&mut self) -> &mut ChaCha8Rng {
        &mut self.simulation.rng
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::{set_auto_pause, AttentionClass};
    use crate::core::entity::EntityRef;
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::{decide_index, run_tick};
    use crate::decisions::decision_system::{
        validate_request_decision, validate_resolve_decision, DecisionError,
    };
    use crate::decisions::{
        DecisionContext, DecisionRequestDraft, DecisionResponse, OperationExceptionReason,
    };
    use crate::delegation::delegation_system::{
        resolve_policy_for_manager, validate_assign_mandate, validate_revise_mandate,
        validate_revoke_mandate, DelegationError, MandateRevisionDraft, PolicySource,
    };
    use crate::delegation::{
        BudgetAuthority, BudgetPeriod, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
    };
    use crate::finance::finance_system::{
        insert_account, resolve_budget_usage, validate_record_transaction,
    };
    use crate::finance::{
        AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
        Money,
    };
    use crate::history::history_system::validate_record_event;
    use crate::history::{HistoryEventDraft, HistoryEventKind};
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
    };
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, EvidenceDraft, EvidenceKind, EvidenceStrength, InvestigationDraft,
    };
    use crate::operations::operation_system::{
        apply_transition, validate_authorize_operation, OperationTransition,
    };
    use crate::operations::{
        OperationApproach, OperationConstraint, OperationContingency, OperationDraft,
        OperationKind, OperationObjective, RoleKind,
    };
    use crate::reports::report_system::validate_record_report;
    use crate::reports::{ReportDraft, ReportEntry, ReportKind};
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{
        designate_player_organization, insert_business, insert_character, insert_neighborhood,
        insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessOwner, CapabilityKind, CharacterDraft, ForcePolicy,
        NeighborhoodDraft, OrganizationDraft, OrganizationKind, PolicyKind, PolicySetting, Rating,
        TraitKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct TestScenario {
        state: AppState,
        operation: crate::core::id::OperationId,
        mandate: crate::core::id::MandateId,
    }

    fn level(value: u8) -> RelationshipLevel {
        RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn make_test_scenario() -> TestScenario {
        let registry = build_registry();
        let mut state = AppState::new(0xC11A_1931);

        let player = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Marrow Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("player organization fixture should validate");
        let rival = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Rosetti Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("rival organization fixture should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Central Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police organization fixture should validate");
        designate_player_organization(&mut state, player)
            .expect("player organization designation should validate");

        let south_ward = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "South Ward".to_owned(),
            },
        )
        .expect("neighborhood fixture should validate");

        let boss = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Joseph Marrow".to_owned(),
                organization: Some(player),
                supervisor: None,
                autonomy: AutonomyLevel::Tight,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Management, rating(86)),
                    (CapabilityKind::Negotiation, rating(74)),
                ]),
                traits: BTreeSet::from([TraitKind::Patient]),
                drives: BTreeMap::new(),
            },
        )
        .expect("boss fixture should validate");
        let lieutenant = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Carlo Venn".to_owned(),
                organization: Some(player),
                supervisor: Some(boss),
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Management, rating(78)),
                    (CapabilityKind::Intimidation, rating(72)),
                ]),
                traits: BTreeSet::from([TraitKind::Ambitious, TraitKind::Secretive]),
                drives: BTreeMap::new(),
            },
        )
        .expect("lieutenant fixture should validate");
        let associate = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Frank Dello".to_owned(),
                organization: Some(player),
                supervisor: Some(lieutenant),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Burglary, rating(77)),
                    (CapabilityKind::Stealth, rating(69)),
                ]),
                traits: BTreeSet::from([TraitKind::EasilyFrightened]),
                drives: BTreeMap::new(),
            },
        )
        .expect("associate fixture should validate");
        let detective = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Detective Harlan".to_owned(),
                organization: Some(police),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(82))]),
                traits: BTreeSet::from([TraitKind::Patient]),
                drives: BTreeMap::new(),
            },
        )
        .expect("detective fixture should validate");
        let rival_recruiter = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Maria Rosetti".to_owned(),
                organization: Some(rival),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(81))]),
                traits: BTreeSet::from([TraitKind::Charismatic]),
                drives: BTreeMap::new(),
            },
        )
        .expect("rival recruiter fixture should validate");

        let garage = insert_business(
            &mut state,
            BusinessDraft {
                name: "Fulton Garage".to_owned(),
                neighborhood: south_ward,
                owner: BusinessOwner::Organization(player),
            },
        )
        .expect("business fixture should validate");

        let budget_funding = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(player),
                kind: AccountKind::AccountedFunds,
                label: "Delegated funding".to_owned(),
            },
        )
        .expect("budget funding account fixture should validate");
        insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(player),
                kind: AccountKind::Payable,
                label: "Delegated expenses".to_owned(),
            },
        )
        .expect("budget destination account fixture should validate");

        validate_set_relationship(
            &state,
            lieutenant,
            boss,
            RelationshipDimensions {
                trust: level(58),
                respect: level(71),
                fear: level(35),
                affection: level(22),
                dependence: level(67),
                resentment: level(48),
                debt: level(15),
            },
        )
        .expect("relationship fixture should validate")
        .commit(&mut state);
        validate_set_relationship(
            &state,
            associate,
            rival_recruiter,
            RelationshipDimensions {
                trust: level(18),
                respect: level(44),
                fear: level(20),
                affection: level(12),
                dependence: level(8),
                resentment: level(5),
                debt: level(0),
            },
        )
        .expect("cross-organization relationship fixture should validate")
        .commit(&mut state);

        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(player),
                source_kind: InformationSourceKind::PoliceContact,
                source_entity: Some(EntityRef::Character(detective)),
                subject: EntityRef::Character(associate),
                observed_at: state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "Central Precinct is asking questions about Frank Dello.".to_owned(),
            },
        )
        .expect("information fixture should validate")
        .commit(&mut state);

        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "South Ward collection assault".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(associate)]),
            },
        )
        .expect("investigation fixture should validate")
        .commit(&mut state);
        validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(associate),
                kind: EvidenceKind::WitnessTestimony,
                strength: EvidenceStrength::Strong,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("evidence fixture should validate")
        .commit(&mut state);

        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Fulton document recovery".to_owned(),
                kind: OperationKind::DocumentTheft,
                responsible_organization: player,
                leader: lieutenant,
                objective: OperationObjective::AcquireProperty {
                    target: EntityRef::Business(garage),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([
                    (RoleKind::Coordinator, lieutenant),
                    (RoleKind::EntrySpecialist, associate),
                ]),
                constraints: vec![OperationConstraint::AvoidCasualties],
                contingencies: vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
                scheduled_for: SimTime::from_minutes(10),
            },
        )
        .expect("operation fixture should validate")
        .commit(&mut state);

        let mandate = validate_assign_mandate(
            &registry,
            &state,
            MandateDraft {
                organization: player,
                manager: lieutenant,
                scopes: BTreeSet::from([
                    ResponsibilityScope::Neighborhood(south_ward),
                    ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ]),
                standing_orders: BTreeMap::from([(
                    PolicyKind::CollectionForce,
                    PolicySetting::CollectionForce(ForcePolicy::None),
                )]),
                budget: Some(BudgetAuthority {
                    funding_account: budget_funding,
                    limit: Money::from_cents(250_000),
                    period: BudgetPeriod::Weekly,
                }),
            },
        )
        .expect("mandate fixture should validate")
        .commit(&mut state)
        .expect("validated mandate should remain current");

        validate_record_report(
            &state,
            ReportDraft {
                recipient: player,
                kind: ReportKind::PoliceIntelligence,
                title: "Police intelligence".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "Detectives are asking about Frank; exact evidence remains unknown."
                        .to_owned(),
                    sources: vec![information],
                    entities: BTreeSet::from([
                        EntityRef::Character(associate),
                        EntityRef::Investigation(investigation),
                    ]),
                    decision: None,
                }],
            },
        )
        .expect("report fixture should validate")
        .commit(&mut state);

        validate_record_event(
            &state,
            HistoryEventDraft {
                occurred_at: state.now(),
                kind: HistoryEventKind::Investigation,
                summary: "Central Precinct opened an investigation touching Frank Dello."
                    .to_owned(),
                entities: BTreeSet::from([
                    EntityRef::Character(associate),
                    EntityRef::Investigation(investigation),
                ]),
            },
        )
        .expect("history fixture should validate")
        .commit(&mut state);

        crate::core::invariants::validate_invariants(&state);
        TestScenario {
            state,
            operation,
            mandate,
        }
    }

    #[test]
    fn test_mixed_scenario_soak_preserves_invariants() {
        let TestScenario {
            mut state,
            operation,
            mandate,
        } = make_test_scenario();

        assert!(state.delegation().get_mandate(mandate).is_some());
        let budget_funding = state
            .delegation()
            .get_mandate(mandate)
            .and_then(|record| record.budget())
            .expect("soak mandate should have budget authority")
            .funding_account;
        let budget_destination = state
            .finance()
            .accounts_for(FinancialOwner::Organization(
                state
                    .player_organization()
                    .expect("fixture should have player organization"),
            ))
            .find(|account| account.kind() == AccountKind::Payable)
            .expect("fixture should have budget destination account")
            .id();

        let mut pending_decision = None;
        for minute in 1..=5_000_u64 {
            let outcome = run_tick(&mut state);
            assert_eq!(outcome.now.as_minutes(), minute);
            match minute {
                10 => assert_eq!(outcome.started_operations, vec![operation]),
                11 => {
                    let leader = state
                        .operations()
                        .get_operation(operation)
                        .expect("operation should exist")
                        .leader();
                    let request = validate_request_decision(
                        &state,
                        DecisionRequestDraft {
                            requester: leader,
                            context: DecisionContext::OperationException {
                                operation,
                                reason: OperationExceptionReason::UnexpectedCondition,
                            },
                            attention: AttentionClass::Exception,
                            summary: "Execution encountered an unexpected condition outside delegated authority."
                                .to_owned(),
                        },
                    )
                    .expect("delegated exception should validate")
                    .commit(&mut state)
                    .expect("validated exception should remain current");
                    assert!(request.requests_pause);
                    let recipient = state
                        .player_organization()
                        .expect("fixture should have player organization");
                    validate_record_report(
                        &state,
                        ReportDraft {
                            recipient,
                            kind: ReportKind::ExecutiveBrief,
                            title: "Decision required".to_owned(),
                            entries: vec![ReportEntry {
                                attention: AttentionClass::Exception,
                                summary: "A delegated operation requires guidance.".to_owned(),
                                sources: Vec::new(),
                                entities: BTreeSet::from([
                                    EntityRef::Operation(operation),
                                    EntityRef::DecisionRequest(request.decision),
                                ]),
                                decision: Some(request.decision),
                            }],
                        },
                    )
                    .expect("decision-linked report should validate")
                    .commit(&mut state);
                    pending_decision = Some(request.decision);
                }
                12 => {
                    let decision = pending_decision
                        .take()
                        .expect("previous tick should have created a decision");
                    let recipient = state
                        .player_organization()
                        .expect("fixture should have player organization");
                    validate_resolve_decision(
                        &state,
                        decision,
                        recipient,
                        DecisionResponse::Continue,
                    )
                    .expect("decision resolution should validate")
                    .commit(&mut state)
                    .expect("validated resolution should remain current");
                }
                13 => apply_transition(&mut state, operation, OperationTransition::Complete)
                    .expect("in-progress operation should complete"),
                20 => {
                    validate_record_transaction(
                        &state,
                        LedgerTransactionDraft {
                            occurred_at: state.now(),
                            memo: "Delegated operating expense".to_owned(),
                            postings: vec![
                                LedgerPosting {
                                    account: budget_funding,
                                    amount: Money::from_cents(-50_000),
                                },
                                LedgerPosting {
                                    account: budget_destination,
                                    amount: Money::from_cents(50_000),
                                },
                            ],
                            authorization: Some(mandate),
                        },
                    )
                    .expect("delegated expense should fit the mandate budget")
                    .commit(&mut state)
                    .expect("validated delegated expense should remain current");
                }
                _ => {}
            }
        }

        let budget = resolve_budget_usage(&state, mandate, state.now())
            .expect("soak mandate budget should remain resolvable");
        assert_eq!(budget.used, Money::from_cents(50_000));
        assert_eq!(budget.remaining, Money::from_cents(200_000));
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn active_operation_assignment_blocks_organization_reassignment() {
        let TestScenario {
            mut state,
            operation,
            mandate: _,
        } = make_test_scenario();
        for _ in 0..10 {
            run_tick(&mut state);
        }
        let participant = *state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .roles()
            .get(&RoleKind::EntrySpecialist)
            .expect("fixture should have entry specialist");
        let version = state
            .world()
            .get_character(participant)
            .expect("participant should exist")
            .version();

        let error = validate_reassign_character(&state, participant, None, None)
            .expect_err("active assignment must prevent organization reassignment");
        assert_eq!(
            error,
            WorldError::ActiveOperationAssignment {
                character: participant,
                operation,
            }
        );
        let participant_record = state
            .world()
            .get_character(participant)
            .expect("participant should still exist");
        assert_eq!(participant_record.version(), version);
        assert_eq!(
            participant_record.organization(),
            state.player_organization()
        );
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn decision_request_rejects_non_interrupting_attention_without_mutation() {
        let TestScenario {
            mut state,
            operation,
            mandate: _,
        } = make_test_scenario();
        for _ in 0..10 {
            run_tick(&mut state);
        }
        let record = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist");
        let leader = record.leader();
        let version = record.version();

        let error = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Notable,
                summary: "A delegated operation encountered an unexpected condition.".to_owned(),
            },
        )
        .expect_err("non-interrupting attention must not create a decision request");

        assert_eq!(error, DecisionError::InvalidAttention);
        let record = state
            .operations()
            .get_operation(operation)
            .expect("operation should still exist");
        assert_eq!(
            record.status(),
            crate::operations::OperationStatus::InProgress
        );
        assert_eq!(record.version(), version);
        assert_eq!(state.decisions().pending_for_operation(operation), None);
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn stale_decision_resolution_cannot_commit_twice() {
        let TestScenario {
            mut state,
            operation,
            mandate: _,
        } = make_test_scenario();
        for _ in 0..10 {
            run_tick(&mut state);
        }
        let leader = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .leader();
        let decision = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: "A delegated operation requires guidance.".to_owned(),
            },
        )
        .expect("decision request should validate")
        .commit(&mut state)
        .expect("validated request should remain current")
        .decision;
        let recipient = state
            .player_organization()
            .expect("fixture should have player organization");
        let current =
            validate_resolve_decision(&state, decision, recipient, DecisionResponse::Continue)
                .expect("first resolution should validate");
        let stale =
            validate_resolve_decision(&state, decision, recipient, DecisionResponse::Continue)
                .expect("second resolution should validate against the same snapshot");

        current
            .commit(&mut state)
            .expect("first resolution should commit");
        let error = stale
            .commit(&mut state)
            .expect_err("stale resolution must not commit twice");
        assert_eq!(
            error,
            DecisionError::StaleDecision {
                decision,
                expected: 1,
                found: 2,
            }
        );
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn mandate_policy_override_falls_back_to_organization_after_revocation() {
        let TestScenario {
            mut state,
            operation: _,
            mandate,
        } = make_test_scenario();
        let mandate_record = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should exist");
        let manager = mandate_record.manager();
        let organization = mandate_record.organization();

        let delegated = resolve_policy_for_manager(&state, manager, PolicyKind::CollectionForce)
            .expect("manager policy should resolve");
        assert_eq!(
            delegated.setting,
            PolicySetting::CollectionForce(ForcePolicy::None)
        );
        assert_eq!(delegated.source, PolicySource::Mandate(mandate));

        validate_revoke_mandate(&state, mandate)
            .expect("active mandate should be revocable")
            .commit(&mut state)
            .expect("validated revocation should remain current");

        let inherited = resolve_policy_for_manager(&state, manager, PolicyKind::CollectionForce)
            .expect("organization policy should resolve after revocation");
        assert_eq!(
            inherited.setting,
            PolicySetting::CollectionForce(ForcePolicy::ThreatsOnly)
        );
        assert_eq!(inherited.source, PolicySource::Organization(organization));
        assert!(state.delegation().active_for_manager(manager).is_none());
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn stale_mandate_revision_cannot_overwrite_newer_revision() {
        let registry = build_registry();
        let TestScenario {
            mut state,
            operation: _,
            mandate,
        } = make_test_scenario();
        let stale = validate_revise_mandate(
            &registry,
            &state,
            mandate,
            MandateRevisionDraft {
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Intelligence,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("first revision should validate");
        let current = validate_revise_mandate(
            &registry,
            &state,
            mandate,
            MandateRevisionDraft {
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Finance,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("second revision should validate against the same snapshot");

        current
            .commit(&mut state)
            .expect("current revision should commit");
        let error = stale
            .commit(&mut state)
            .expect_err("stale mandate revision must not overwrite newer state");
        assert_eq!(
            error,
            DelegationError::StaleMandate {
                mandate,
                expected: 1,
                found: 2,
            }
        );
        let record = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should still exist");
        assert!(record.scopes().contains(&ResponsibilityScope::Function(
            ResponsibilityFunction::Finance
        )));
        assert!(!record.scopes().contains(&ResponsibilityScope::Function(
            ResponsibilityFunction::Intelligence
        )));
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn active_mandate_blocks_manager_organization_reassignment() {
        let TestScenario {
            state,
            operation: _,
            mandate,
        } = make_test_scenario();
        let manager = state
            .delegation()
            .get_mandate(mandate)
            .expect("mandate should exist")
            .manager();
        let version = state
            .world()
            .get_character(manager)
            .expect("manager should exist")
            .version();

        let error = validate_reassign_character(&state, manager, None, None)
            .expect_err("active mandate must prevent organization reassignment");
        assert_eq!(
            error,
            WorldError::ActiveMandateAssignment {
                character: manager,
                mandate,
            }
        );
        assert_eq!(
            state
                .world()
                .get_character(manager)
                .expect("manager should still exist")
                .version(),
            version
        );
        crate::core::invariants::validate_invariants(&state);
    }

    #[test]
    fn save_round_trip_preserves_pending_decision_and_attention_settings() {
        let TestScenario {
            mut state,
            operation,
            mandate: _,
        } = make_test_scenario();
        for _ in 0..10 {
            run_tick(&mut state);
        }

        set_auto_pause(&mut state, AttentionClass::Exception, false);
        set_auto_pause(&mut state, AttentionClass::Notable, true);
        let leader = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .leader();
        let request = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: "Unexpected condition requires guidance.".to_owned(),
            },
        )
        .expect("pending decision should validate")
        .commit(&mut state)
        .expect("validated pending decision should remain current");
        assert!(!request.requests_pause);

        let recipient = state
            .player_organization()
            .expect("fixture should have player organization");
        let report = validate_record_report(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::ExecutiveBrief,
                title: "Pending guidance".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Exception,
                    summary: "A pending decision requires guidance.".to_owned(),
                    sources: Vec::new(),
                    entities: BTreeSet::from([
                        EntityRef::Operation(operation),
                        EntityRef::DecisionRequest(request.decision),
                    ]),
                    decision: Some(request.decision),
                }],
            },
        )
        .expect("pending-decision report should validate")
        .commit(&mut state);

        let registry = build_registry();
        let envelope = build_save(&registry, &state).expect("valid pending state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored = restore_save(&registry, decoded).expect("pending save should restore");

        assert!(!restored
            .attention_settings()
            .is_auto_pause_enabled(AttentionClass::Exception));
        assert!(restored
            .attention_settings()
            .is_auto_pause_enabled(AttentionClass::Notable));
        assert_eq!(
            restored
                .operations()
                .get_operation(operation)
                .expect("restored operation should exist")
                .status(),
            crate::operations::OperationStatus::AwaitingDecision
        );
        assert_eq!(
            restored.decisions().pending_for_operation(operation),
            Some(request.decision)
        );
        assert_eq!(
            restored
                .decisions()
                .get_decision(request.decision)
                .expect("restored decision should exist")
                .status(),
            crate::decisions::DecisionStatus::Pending
        );
        assert_eq!(
            restored
                .reports()
                .get_report(report)
                .expect("restored report should exist")
                .entries()[0]
                .decision,
            Some(request.decision)
        );

        validate_resolve_decision(
            &restored,
            request.decision,
            recipient,
            DecisionResponse::Continue,
        )
        .expect("restored decision should remain resolvable")
        .commit(&mut restored)
        .expect("restored decision resolution should commit");
        assert_eq!(
            restored
                .operations()
                .get_operation(operation)
                .expect("restored operation should still exist")
                .status(),
            crate::operations::OperationStatus::InProgress
        );
        assert_eq!(
            restored
                .decisions()
                .get_decision(request.decision)
                .expect("resolved decision should still exist")
                .status(),
            crate::decisions::DecisionStatus::Resolved
        );
        crate::core::invariants::validate_invariants(&restored);
    }

    #[test]
    fn save_round_trip_preserves_deterministic_continuation() {
        let TestScenario {
            mut state,
            operation: _,
            mandate: _,
        } = make_test_scenario();
        for _ in 0..37 {
            run_tick(&mut state);
        }
        for _ in 0..16 {
            decide_index(&mut state, 23).expect("non-empty random choice should resolve");
        }

        let registry = build_registry();
        let envelope = build_save(&registry, &state).expect("valid state should build a save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored = restore_save(&registry, decoded).expect("current save should restore");

        assert_eq!(restored.now(), state.now());
        for _ in 0..256 {
            assert_eq!(
                decide_index(&mut state, 97).expect("choice should resolve"),
                decide_index(&mut restored, 97).expect("restored choice should resolve")
            );
        }
    }
}
