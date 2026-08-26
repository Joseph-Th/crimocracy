//! Soak-fixture and persistence/continuation tests for `core::state`.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::IdKind;
use crate::core::invariants::{validate_state, StateValidationError};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::simulation::run_tick;
use crate::decisions::decision_system::{
    validate_request_recruitment_approval, validate_resolve_decision, DecisionError,
};
use crate::decisions::{DecisionResponse, RecruitmentApprovalRequestDraft};
use crate::delegation::delegation_system::{
    resolve_policy_for_manager, validate_assign_mandate, validate_revise_mandate,
    validate_revoke_mandate, DelegationError, MandateRevisionDraft, PolicySource,
};
use crate::delegation::{
    BudgetAuthority, BudgetPeriod, MandateAuthority, MandateDraft, ResponsibilityFunction,
    ResponsibilityScope,
};
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::economy::BusinessEconomyDraft;
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::finance::finance_system::{
    insert_account, resolve_budget_usage, validate_record_transaction,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::history_system::validate_record_event;
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    validate_information_transfer, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTransferDraft, KnowledgeHolder,
    Reliability, Specificity,
};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::{
    Admissibility, EvidenceDraft, EvidenceKind, EvidenceStrength, InvestigationDraft,
    JurisdictionDraft,
};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationContingency, OperationDraft, OperationKind, OperationObjective,
    RoleKind,
};
use crate::recruitment::recruitment_system::validate_recruitment_attempt;
use crate::recruitment::{RecruitmentApproach, RecruitmentDraft, RecruitmentOutcome};
use crate::reports::organization_financial_report::validate_organization_financial_report;
use crate::reports::report_system::validate_record_report;
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization, validate_reassign_character, WorldError,
};
use crate::world::{
    ApprovalPolicy, AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
    CapabilityKind, CharacterDraft, DriveKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    PolicyKind, PolicySetting, Rating, TraitKind,
};
use rand_core::RngCore;
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
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(45),
                    commercial_activity: rating(70),
                    illicit_demand: rating(72),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(58),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([south_ward]),
            case_intake_priority: rating(80),
        },
    )
    .expect("precinct jurisdiction fixture should validate")
    .commit(&mut state)
    .expect("precinct jurisdiction fixture should commit");

    let boss = insert_character(
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
            drives: BTreeMap::from([(DriveKind::Safety, rating(90))]),
        },
    )
    .expect("associate fixture should validate");
    let detective = insert_character(
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
        &registry,
        &mut state,
        BusinessDraft {
            name: "Fulton Garage".to_owned(),
            kind: BusinessKind::Automotive,
            functions: BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood: south_ward,
            // An independent owner: an organization cannot target its own premises.
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business fixture should validate");
    let business_operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(garage),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("business operating account fixture should validate");
    let business_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(garage),
            kind: AccountKind::Settlement,
        },
    )
    .expect("business settlement account fixture should validate");
    validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business: garage,
            operating_account: business_operating,
            settlement_account: business_settlement,
        },
    )
    .expect("business economy fixture should validate")
    .commit(&mut state)
    .expect("business economy fixture should commit");

    let budget_funding = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("budget funding account fixture should validate");
    insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player),
            kind: AccountKind::AccountedFunds,
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

    let field_information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(lieutenant),
            source_kind: InformationSourceKind::PoliceContact,
            topic: crate::intelligence::InformationTopic::PoliceActivity,
            source_entity: Some(EntityRef::Character(detective)),
            subject: EntityRef::Character(associate),
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Central Precinct is asking questions about Frank Dello.".to_owned(),
        },
    )
    .expect("field information fixture should validate")
    .commit(&mut state)
    .expect("field information fixture should commit");
    let information = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source: field_information,
            recipient: KnowledgeHolder::Organization(player),
        },
    )
    .expect("member information should transfer into organization knowledge")
    .commit(&mut state)
    .expect("validated organization information transfer should commit");
    validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(associate),
            source_kind: InformationSourceKind::PoliceContact,
            topic: crate::intelligence::InformationTopic::PoliceActivity,
            source_entity: Some(EntityRef::Character(detective)),
            subject: EntityRef::Character(associate),
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Frank Dello knows detectives are asking questions specifically about him."
                .to_owned(),
        },
    )
    .expect("associate legal-pressure knowledge should validate")
    .commit(&mut state)
    .expect("associate legal-pressure knowledge should commit");

    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "South Ward collection assault".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(associate)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("validated investigation fixture should commit");
    validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(associate),
            origin: None,
            kind: EvidenceKind::WitnessTestimony,
            strength: EvidenceStrength::Strong,
            reliability: crate::legal::EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("evidence fixture should validate")
    .commit(&mut state)
    .expect("validated evidence fixture should commit");

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
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: vec![OperationContingency::RequestDecisionOnPoliceArrival],
            scheduled_for: SimTime::from_minutes(10),
        },
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("validated operation should remain current");

    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: player,
            manager: lieutenant,
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(south_ward),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
            ]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
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
            kind: ReportKind::Legal,
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
    .commit(&mut state)
    .expect("report fixture should commit");

    validate_record_event(
        &state,
        HistoryEventDraft {
            occurred_at: state.now(),
            kind: HistoryEventKind::Operation,
            summary: "Central Precinct opened an investigation touching Frank Dello.".to_owned(),
            entities: BTreeSet::from([
                EntityRef::Character(associate),
                EntityRef::Investigation(investigation),
            ]),
        },
    )
    .expect("history fixture should validate")
    .commit(&mut state)
    .expect("history fixture should commit");

    crate::core::invariants::validate_invariants(&state);
    TestScenario {
        state,
        operation,
        mandate,
    }
}

#[test]
fn test_mixed_scenario_soak_preserves_invariants() {
    let registry = build_registry();
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
        .find(|account| {
            account.kind() == AccountKind::AccountedFunds && account.id() != budget_funding
        })
        .expect("fixture should have budget destination account")
        .id();
    let budget_authorization = MandateAuthority {
        mandate,
        manager: state
            .delegation()
            .get_mandate(mandate)
            .expect("soak mandate should exist")
            .manager(),
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Operations),
    };
    let enterprise_neighborhood = state
        .delegation()
        .get_mandate(mandate)
        .expect("soak mandate should exist")
        .scopes()
        .iter()
        .find_map(|scope| match scope {
            ResponsibilityScope::Neighborhood(id) => Some(*id),
            ResponsibilityScope::Business(_) | ResponsibilityScope::Function(_) => None,
        })
        .expect("soak mandate should contain a neighborhood scope");
    let player_organization = state
        .player_organization()
        .expect("fixture should have player organization");
    let enterprise_cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player_organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("enterprise cash account should validate");
    let enterprise_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(player_organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("enterprise settlement account should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: player_organization,
            authority: MandateAuthority {
                mandate,
                manager: budget_authorization.manager,
                scope: ResponsibilityScope::Neighborhood(enterprise_neighborhood),
            },
            location: EnterpriseLocation::Neighborhood(enterprise_neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: enterprise_cash,
            settlement_account: enterprise_settlement,
        },
    )
    .expect("delegated routine enterprise should validate")
    .commit(&mut state)
    .expect("delegated routine enterprise should commit");

    let mut rival_recruitment = None;
    for minute in 1..=5_000_u64 {
        let outcome = run_tick(&registry, &mut state);
        assert_eq!(outcome.now.as_minutes(), minute);
        match minute {
            10 => assert_eq!(outcome.started_operations, vec![operation]),
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
                        authorization: Some(budget_authorization),
                    },
                )
                .expect("delegated expense should fit the mandate budget")
                .commit(&mut state)
                .expect("validated delegated expense should remain current");
            }
            60 => {
                let candidate = *state
                    .operations()
                    .get_operation(operation)
                    .expect("operation should persist through recruitment")
                    .roles()
                    .get(&RoleKind::EntrySpecialist)
                    .expect("fixture should have a recruitable entry specialist");
                let recruiter = state
                    .social()
                    .relationships()
                    .filter(|relationship| relationship.from() == candidate)
                    .find_map(|relationship| {
                        let contact = state.world().get_character(relationship.to())?;
                        (contact.organization() != Some(player_organization))
                            .then_some(contact.id())
                    })
                    .expect("fixture should expose a relational rival recruiter");
                let target_organization = state
                    .world()
                    .get_character(recruiter)
                    .and_then(|character| character.organization())
                    .expect("rival recruiter should belong to an organization");
                let attempt = validate_recruitment_attempt(
                    &registry,
                    &state,
                    RecruitmentDraft {
                        target_organization,
                        recruiter,
                        candidate,
                        approach: RecruitmentApproach::Protection,
                    },
                )
                .expect("rival protection recruitment should validate after operation completion")
                .commit(&mut state)
                .expect("rival recruitment should commit through the canonical personnel path");
                assert_eq!(
                    state
                        .recruitment()
                        .get_attempt(attempt)
                        .expect("rival recruitment should persist")
                        .outcome(),
                    RecruitmentOutcome::Accepted
                );
                rival_recruitment = Some((attempt, candidate, target_organization));
            }
            1_440 | 2_880 | 4_320 => {
                assert_eq!(outcome.business_cycles.len(), 1);
                assert_eq!(outcome.enterprise_cycles.len(), 1);
            }
            _ => {}
        }

        for request in outcome.decision_requests {
            assert!(
                request.requests_pause,
                "automatic operation decisions must surface the existing auto-pause contract"
            );
            // Leadership sees the surfaced exception through a canonical player-visible
            // report linked to the decision before resolving it.
            let recipient = state
                .decisions()
                .get_decision(request.decision)
                .expect("tick-generated decision should persist before resolution")
                .recipient();
            validate_record_report(
                &state,
                ReportDraft {
                    recipient,
                    kind: ReportKind::Legal,
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
            .commit(&mut state)
            .expect("decision-linked report should commit");
            validate_resolve_decision(
                &registry,
                &state,
                request.decision,
                recipient,
                DecisionResponse::Continue,
            )
            .expect("soak adapter should be able to continue a surfaced operation decision")
            .commit(&mut state)
            .expect("surfaced operation decision should resolve through the canonical path");
        }
    }

    let budget = resolve_budget_usage(&state, mandate, state.now())
        .expect("soak mandate budget should remain resolvable");
    assert_eq!(budget.used, Money::from_cents(50_000));
    assert_eq!(budget.remaining, Money::from_cents(200_000));
    assert_eq!(state.economy().cycles().count(), 3);
    assert_eq!(state.enterprises().cycles_for(enterprise).count(), 3);
    let (recruitment_attempt, recruited_candidate, rival_organization) = rival_recruitment
        .expect("soak should exercise rival recruitment under known legal pressure");
    let recruitment = state
        .recruitment()
        .get_attempt(recruitment_attempt)
        .expect("soak recruitment should persist");
    assert_eq!(recruitment.outcome(), RecruitmentOutcome::Accepted);
    assert!(recruitment.pressure_information().is_some());
    assert!(recruitment.factors().perceived_legal_pressure() > 0);
    assert_eq!(
        state
            .world()
            .get_character(recruited_candidate)
            .expect("recruited candidate should persist")
            .organization(),
        Some(rival_organization)
    );
    let operation_resolution = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("soak operation should have resolved causally");
    assert_eq!(
        operation_resolution.exposure().neighborhood(),
        Some(enterprise_neighborhood)
    );
    let operation_investigation = operation_resolution
        .exposure()
        .investigation()
        .expect("soak operation exposure should route into the precinct jurisdiction");
    let investigation_owner = state
        .legal()
        .get_investigation(operation_investigation)
        .expect("soak operation investigation should persist")
        .owner();
    assert!(matches!(
        state
            .world()
            .get_organization(investigation_owner)
            .expect("operation investigation owner should exist")
            .kind(),
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    ));
    let operation_evidence = operation_resolution
        .exposure()
        .evidence()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(operation_evidence.len(), 1);
    assert_eq!(
        state
            .legal()
            .get_evidence(operation_evidence[0])
            .expect("soak operation evidence should persist")
            .origin(),
        Some(EntityRef::Operation(operation))
    );
    let enterprise_net = state
        .enterprises()
        .cycles_for(enterprise)
        .try_fold(Money::ZERO, |total, cycle| {
            total.checked_add(cycle.net_cash())
        })
        .expect("soak enterprise cycle totals should not overflow");
    assert_eq!(
        state
            .finance()
            .get_account(enterprise_cash)
            .expect("enterprise cash account should exist")
            .balance(),
        enterprise_net
    );
    assert_eq!(
        state
            .finance()
            .get_account(enterprise_settlement)
            .expect("enterprise settlement account should exist")
            .balance(),
        Money::from_cents(
            enterprise_net
                .cents()
                .checked_neg()
                .expect("enterprise soak net should be negatable")
        )
    );
    let financial_report = validate_organization_financial_report(
        &state,
        state
            .player_organization()
            .expect("fixture should have player organization"),
        SimTime::ZERO,
        state.now(),
    )
    .expect("combined organization financial report should validate after soak")
    .commit(&mut state)
    .expect("combined organization financial report should commit");
    let financial_report = state
        .reports()
        .get_report(financial_report)
        .expect("combined organization financial report should persist");
    assert_eq!(financial_report.kind(), ReportKind::Financial);
    assert!(!financial_report.entries().is_empty());
    crate::core::invariants::validate_invariants(&state);
}

#[test]
fn active_operation_assignment_blocks_organization_reassignment() {
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation,
        mandate: _,
    } = make_test_scenario();
    for _ in 0..10 {
        run_tick(&registry, &mut state);
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
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation,
        mandate,
    } = make_test_scenario();
    for _ in 0..10 {
        run_tick(&registry, &mut state);
    }
    let record = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist");
    let leader = record.leader();
    let version = record.version();

    // Attention metadata is validated before any recruitment-specific precondition, so a
    // non-interrupting class is rejected without touching authoritative state.
    let error = validate_request_recruitment_approval(
        &registry,
        &state,
        RecruitmentApprovalRequestDraft {
            authority: MandateAuthority {
                mandate,
                manager: leader,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
            },
            target_organization: record.responsible_organization(),
            recruiter: leader,
            candidate: leader,
            approach: RecruitmentApproach::Protection,
            attention: AttentionClass::Notable,
            summary: "A delegated operation requires guidance.".to_owned(),
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
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation: _,
        mandate: _,
    } = make_test_scenario();
    // The operation's police response arrives and pauses it with a pending decision.
    run_tick(&registry, &mut state);
    let decision = loop {
        let outcome = run_tick(&registry, &mut state);
        if let Some(request) = outcome.decision_requests.first() {
            break request.decision;
        }
    };
    let recipient = state
        .player_organization()
        .expect("fixture should have player organization");
    let current = validate_resolve_decision(
        &registry,
        &state,
        decision,
        recipient,
        DecisionResponse::Continue,
    )
    .expect("first resolution should validate");
    let stale = validate_resolve_decision(
        &registry,
        &state,
        decision,
        recipient,
        DecisionResponse::Continue,
    )
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

    let delegated = resolve_policy_for_manager(&state, manager, PolicyKind::IndependentRecruitment)
        .expect("manager policy should resolve");
    assert_eq!(
        delegated.setting,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated)
    );
    assert_eq!(delegated.source, PolicySource::Mandate(mandate));

    validate_revoke_mandate(&state, mandate)
        .expect("active mandate should be revocable")
        .commit(&mut state)
        .expect("validated revocation should remain current");

    let inherited = resolve_policy_for_manager(&state, manager, PolicyKind::IndependentRecruitment)
        .expect("organization policy should resolve after revocation");
    assert_eq!(
        inherited.setting,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval)
    );
    assert_eq!(inherited.source, PolicySource::Organization(organization));
    assert!(state.delegation().active_for_manager(manager).is_none());
    crate::core::invariants::validate_invariants(&state);
}

#[test]
fn stale_mandate_revision_cannot_overwrite_newer_revision() {
    let TestScenario {
        mut state,
        operation: _,
        mandate,
    } = make_test_scenario();
    let stale = validate_revise_mandate(
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
    validate_state(&state).expect("fixture state should be structurally valid before rejection");
    crate::core::invariants::validate_invariants(&state);
    let state_before =
        bincode::serialize(&state).expect("fixture state should serialize before rejection");

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
    assert_eq!(
        bincode::serialize(&state).expect("rejected state should still serialize"),
        state_before,
        "rejected reassignment must leave the complete persisted state unchanged"
    );
    validate_state(&state).expect("rejected operation must preserve structural validity");
    crate::core::invariants::validate_invariants(&state);
}

#[test]
fn state_validation_rejects_rewound_id_allocator() {
    let registry = build_registry();
    let mut state = AppState::new(0x1D_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Allocator Validation Organization".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("organization fixture should validate");

    state
        .ids
        .set_next_raw_for_test(IdKind::Organization, organization.raw());
    assert_eq!(
        validate_state(&state),
        Err(StateValidationError::InvalidIdAllocator {
            kind: "organization",
            next: organization.raw(),
            highest: organization.raw(),
        })
    );
}

#[test]
fn save_round_trip_preserves_pending_decision_and_attention_settings() {
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation,
        mandate: _,
    } = make_test_scenario();
    for _ in 0..10 {
        run_tick(&registry, &mut state);
    }

    // Attention preferences persist with the campaign envelope so a restored campaign
    // keeps its pause behavior.
    let default_auto_pause = state.attention_settings().clone();
    // The operation's police response arrives and raises the exception decision through
    // the canonical arrival path.
    let request = loop {
        let outcome = run_tick(&registry, &mut state);
        if let Some(decision) = outcome.decision_requests.first() {
            break *decision;
        }
    };
    // Exception-class requests pause by default.
    assert!(request.requests_pause);

    let recipient = state
        .player_organization()
        .expect("fixture should have player organization");
    let report = validate_record_report(
        &state,
        ReportDraft {
            recipient,
            kind: ReportKind::Legal,
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
    .commit(&mut state)
    .expect("pending-decision report should commit");

    // Toggle both interrupting classes off their defaults before saving so the round trip
    // proves changed preferences, not just restored defaults.
    state.set_auto_pause(AttentionClass::Exception, false);
    state.set_auto_pause(AttentionClass::Crisis, false);

    let envelope = build_save(&registry, &state).expect("valid pending state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&registry, decoded).expect("pending save should restore");

    assert!(
        !restored
            .attention_settings()
            .is_auto_pause_enabled(AttentionClass::Exception),
        "toggled-off Exception auto-pause must survive save/load"
    );
    assert!(
        !restored
            .attention_settings()
            .is_auto_pause_enabled(AttentionClass::Crisis),
        "toggled-off Crisis auto-pause must survive save/load"
    );
    assert_ne!(
        default_auto_pause.is_auto_pause_enabled(AttentionClass::Crisis),
        restored
            .attention_settings()
            .is_auto_pause_enabled(AttentionClass::Crisis)
    );
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
        &registry,
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
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation: _,
        mandate: _,
    } = make_test_scenario();
    for _ in 0..37 {
        run_tick(&registry, &mut state);
    }
    for _ in 0..16 {
        // Advance a domain stream so the save carries consumed RNG positions.
        state.operation_rng_mut().next_u64();
    }
    validate_state(&state).expect("pre-save state should be structurally valid");
    crate::core::invariants::validate_invariants(&state);
    let state_before =
        bincode::serialize(&state).expect("pre-save application state should serialize");

    let envelope = build_save(&registry, &state).expect("valid state should build a save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&registry, decoded).expect("current save should restore");

    assert_eq!(restored.now(), state.now());
    assert_eq!(
        bincode::serialize(&restored).expect("restored application state should serialize"),
        state_before,
        "save/load must reproduce the complete persisted application state"
    );
    validate_state(&restored).expect("restored state should remain structurally valid");
    crate::core::invariants::validate_invariants(&restored);
    for _ in 0..256 {
        assert_eq!(
            state.operation_rng_mut().next_u64(),
            restored.operation_rng_mut().next_u64(),
            "operation RNG stream must continue identically after restore"
        );
        assert_eq!(
            state.investigation_rng_mut().next_u64(),
            restored.investigation_rng_mut().next_u64(),
            "investigation RNG stream must continue identically after restore"
        );
        assert_eq!(
            state.business_rng_mut().next_u64(),
            restored.business_rng_mut().next_u64(),
            "business RNG stream must continue identically after restore"
        );
        assert_eq!(
            state.enterprise_rng_mut().next_u64(),
            restored.enterprise_rng_mut().next_u64(),
            "enterprise RNG stream must continue identically after restore"
        );
    }
}

#[test]
fn save_restore_near_id_exhaustion_remains_recoverable() {
    let registry = build_registry();
    let TestScenario {
        mut state,
        operation: _,
        mandate: _,
    } = make_test_scenario();
    // Put the character counter at its last representable value so a restore keeps exactly
    // that boundary rather than synthesizing capacity or wrapping.
    state
        .ids
        .set_next_raw_for_test(IdKind::Character, u32::MAX - 1);
    let envelope = build_save(&registry, &state).expect("near-exhaustion state should save");
    let bytes = bincode::serialize(&envelope).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored =
        restore_save(&registry, decoded).expect("near-exhaustion save should restore");
    // The last representable allocation still succeeds after restore...
    let last = restored
        .ids
        .next_character()
        .expect("last representable character ID should allocate after restore");
    assert_eq!(last.raw(), u32::MAX - 1);
    // ...and the following allocation is a typed recoverable error, not a panic.
    let error = restored
        .ids
        .next_character()
        .expect_err("exhaustion after restore must be a typed error");
    assert!(matches!(
        error,
        crate::core::id::IdExhaustionError::Exhausted {
            kind: "character",
            next: u32::MAX
        }
    ));
    validate_state(&restored).expect("restored near-exhaustion state must remain valid");
}
