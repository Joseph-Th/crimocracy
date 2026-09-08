//! Focused tests for `enterprise_execution` lifecycle, settlement, and reporting.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::delegation::delegation_system::{
    DelegationError, MandateRevisionDraft, validate_assign_mandate, validate_revise_mandate,
    validate_revoke_mandate,
};
use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
use crate::enterprises::EnterpriseKind;
use crate::enterprises::autonomous_expansion::apply_due_autonomous_enterprises;
use crate::enterprises::enterprise_reporting::{
    resolve_enterprise_financial_summary, resolve_neighborhood_enterprise_financial_summary,
    resolve_organization_enterprise_financial_summary,
};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
};
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{
    validate_add_evidence, validate_incident_intake, validate_open_investigation,
};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft, JurisdictionDraft,
};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
};
use crate::world::world_system::{
    WorldError, insert_business, insert_character, insert_neighborhood, insert_organization,
    validate_reassign_character, validate_transfer_business_ownership,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    OrganizationDraft, OrganizationKind,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct EnterpriseFixture {
    state: AppState,
    authority: MandateAuthority,
    organization: OrganizationId,
    location: EnterpriseLocation,
    cash: FinancialAccountId,
    settlement: FinancialAccountId,
}

#[derive(Clone, Serialize)]
struct LedgerTransactionRecordWire {
    id: crate::core::id::LedgerTransactionId,
    occurred_at: SimTime,
    memo: String,
    postings: Vec<crate::finance::LedgerPosting>,
    budget_usage: Option<crate::finance::BudgetUsageRecord>,
}

fn ledger_transaction_wire(
    record: &crate::finance::LedgerTransactionRecord,
) -> LedgerTransactionRecordWire {
    LedgerTransactionRecordWire {
        id: record.id(),
        occurred_at: record.occurred_at(),
        memo: record.memo().to_owned(),
        postings: record.postings().to_vec(),
        budget_usage: record.budget_usage(),
    }
}

fn replace_serialized_transaction(
    envelope: SaveEnvelope,
    original: &crate::finance::LedgerTransactionRecord,
    replacement: &LedgerTransactionRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("ledger transaction should serialize");
    let mirror = ledger_transaction_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("ledger transaction mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement ledger transaction should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized transaction must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout ledger transaction corruption must remain decodable")
}

fn fund_enterprise_fixture_cash(fixture: &mut EnterpriseFixture, cents: i64) {
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund autonomous expansion fixture".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.settlement,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: fixture.cash,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )
    .expect("fixture funding should validate")
    .commit(&mut fixture.state)
    .expect("fixture funding should commit");
}

#[test]
fn restore_rejects_nonincreasing_enterprise_cycle_time_in_sequential_id_order() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let settle = |fixture: &mut EnterpriseFixture| {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        validate_enterprise_cycle_plan(
            &fixture.state,
            decide_enterprise_cycle(
                &registry,
                &fixture.state,
                enterprise,
                EnterpriseCycleRandomness::new(0, u16::MAX),
            )
            .expect("routine enterprise cycle should decide"),
        )
        .expect("routine enterprise cycle should validate")
        .commit(&mut fixture.state)
        .expect("routine enterprise cycle should commit")
    };
    let first_id = settle(&mut fixture);
    let second_id = settle(&mut fixture);
    let first = fixture
        .state
        .enterprises()
        .get_cycle(first_id)
        .expect("first enterprise cycle should persist");
    let second = fixture
        .state
        .enterprises()
        .get_cycle(second_id)
        .expect("second enterprise cycle should persist");
    assert!(first.id() < second.id());
    assert!(first.occurred_at() < second.occurred_at());
    assert_eq!(first.attention(), AttentionClass::Routine);
    assert!(first.information().is_none());
    let first_transaction_id = first
        .transaction()
        .expect("positive protection cycle should carry a ledger settlement");
    let first_transaction = fixture
        .state
        .finance()
        .get_transaction(first_transaction_id)
        .expect("first enterprise settlement transaction should persist");

    let mut corrupted_cycle = enterprise_cycle_wire(first);
    corrupted_cycle.context.occurred_at = second.occurred_at();
    let mut corrupted_transaction = ledger_transaction_wire(first_transaction);
    corrupted_transaction.occurred_at = second.occurred_at();
    let envelope = replace_serialized_cycle(
        build_save(&registry, &fixture.state)
            .expect("valid two-cycle enterprise should save before chronology corruption"),
        first,
        &corrupted_cycle,
    );
    let envelope =
        replace_serialized_transaction(envelope, first_transaction, &corrupted_transaction);
    let error = restore_save(&registry, envelope)
        .expect_err("nonincreasing per-enterprise cycle history must fail restore");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidEnterpriseCycle { cycle }
        ) if cycle == second_id
    ));
}

#[derive(Clone, Serialize)]
struct EnterpriseCycleContextWire {
    enterprise: EnterpriseId,
    occurred_at: SimTime,
}

#[derive(Clone, Serialize)]
struct EnterpriseCycleFinancialsWire {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    investigation_heat: Money,
}

#[derive(Clone, Serialize)]
struct EnterpriseCycleArtifactsWire {
    attention: AttentionClass,
    drew_vice_attention: bool,
}

#[derive(Clone, Serialize)]
struct EnterpriseCycleProvenanceWire {
    transaction: Option<crate::core::id::LedgerTransactionId>,
    information: Option<crate::core::id::InformationId>,
    vice_information: Option<crate::core::id::InformationId>,
}

#[derive(Clone, Serialize)]
struct EnterpriseCycleRecordWire {
    id: EnterpriseCycleId,
    context: EnterpriseCycleContextWire,
    financials: EnterpriseCycleFinancialsWire,
    artifacts: EnterpriseCycleArtifactsWire,
    provenance: EnterpriseCycleProvenanceWire,
}

fn enterprise_cycle_wire(
    record: &crate::enterprises::EnterpriseCycleRecord,
) -> EnterpriseCycleRecordWire {
    EnterpriseCycleRecordWire {
        id: record.id(),
        context: EnterpriseCycleContextWire {
            enterprise: record.enterprise(),
            occurred_at: record.occurred_at(),
        },
        financials: EnterpriseCycleFinancialsWire {
            gross_revenue: record.gross_revenue(),
            operating_cost: record.operating_cost(),
            net_cash: record.net_cash(),
            variance_basis_points: record.variance_basis_points(),
            investigation_heat: record.investigation_heat(),
        },
        artifacts: EnterpriseCycleArtifactsWire {
            attention: record.attention(),
            drew_vice_attention: record.drew_vice_attention(),
        },
        provenance: EnterpriseCycleProvenanceWire {
            transaction: record.transaction(),
            information: record.information(),
            vice_information: record.vice_information(),
        },
    }
}

fn replace_serialized_cycle(
    envelope: SaveEnvelope,
    original: &crate::enterprises::EnterpriseCycleRecord,
    replacement: &EnterpriseCycleRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("enterprise cycle should serialize");
    let mirror = enterprise_cycle_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("enterprise cycle mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement enterprise cycle should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized cycle must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout enterprise cycle corruption must remain decodable")
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_test_enterprise_fixture() -> EnterpriseFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xE17E_1931);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Enterprise Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Market Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Enterprise Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager fixture should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate fixture should validate")
    .commit(&mut state)
    .expect("mandate fixture should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("cash account fixture should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account fixture should validate");
    EnterpriseFixture {
        state,
        authority: MandateAuthority {
            mandate,
            manager,
            scope: ResponsibilityScope::Neighborhood(neighborhood),
        },
        organization,
        location: EnterpriseLocation::Neighborhood(neighborhood),
        cash,
        settlement,
    }
}

fn establish_protection(registry: &Registry, fixture: &mut EnterpriseFixture) -> EnterpriseId {
    validate_establish_enterprise(
        registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("enterprise fixture should validate")
    .commit(&mut fixture.state)
    .expect("enterprise fixture should commit")
}

fn insert_support_business(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    name: &str,
    kind: BusinessKind,
    functions: BTreeSet<BusinessFunction>,
    owner: BusinessOwner,
) -> BusinessId {
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    insert_business(
        registry,
        &mut fixture.state,
        BusinessDraft {
            name: name.to_owned(),
            kind,
            functions,
            neighborhood,
            owner,
        },
    )
    .expect("support business fixture should validate")
}

fn alcohol_support_network(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
) -> (BusinessId, BusinessId) {
    let transport = insert_support_business(
        registry,
        fixture,
        "Harbor Freight & Storage",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(fixture.organization),
    );
    let retail = insert_support_business(
        registry,
        fixture,
        "Neighborhood Bottle Shop",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(fixture.organization),
    );
    (transport, retail)
}

fn establish_alcohol_distribution(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    supporting_businesses: BTreeSet<BusinessId>,
) -> Result<EnterpriseId, EnterpriseError> {
    validate_establish_enterprise(
        registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses,
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )?
    .commit(&mut fixture.state)
}

#[test]
fn a_drawn_vice_inquiry_settles_notable_and_stays_registry_valid_across_save() {
    // Regression: the persisted notability rule includes drawn vice attention, but the
    // registry-relative invariant re-derivation omitted that disjunct — so a settlement
    // whose only notability trigger was a successful visibility roll committed Notable and
    // then failed every later save/load against the authored content.
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Vice Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .expect("jurisdiction fixture should validate")
    .commit(&mut fixture.state)
    .expect("jurisdiction fixture should commit");
    // Sustained district casework: one active originated neighborhood case sits across BOTH
    // settlements, so the street-heat surcharge is identical in each without already being
    // the dedicated enterprise inquiry this test expects the visibility roll to open.
    let _standing_inquiry = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: police,
            title: "Standing district inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Neighborhood(neighborhood)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Neighborhood(neighborhood),
                origin: Some(EntityRef::Enterprise(enterprise)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Enterprise(enterprise)),
            notified_organizations: BTreeSet::from([fixture.organization]),
            witness: None,
        },
    )
    .expect("standing vice inquiry should validate")
    .commit(&mut fixture.state)
    .expect("standing vice inquiry should commit");

    // First settlement under sustained casework, roll misses: notable through first-time
    // street heat only.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("first due cycle should resolve"),
    )
    .expect("first cycle plan should validate")
    .commit(&mut fixture.state)
    .expect("first cycle should settle");
    let first_record = fixture
        .state
        .enterprises()
        .get_cycle(first)
        .expect("first cycle should exist");
    assert!(!first_record.drew_vice_attention());

    // Second settlement, same case pressure, roll hits: the drawn vice inquiry is the ONLY
    // fresh notability trigger — variance is zero, heat is unchanged, and the book earns.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, 0),
        )
        .expect("second due cycle should resolve"),
    )
    .expect("second cycle plan should validate")
    .commit(&mut fixture.state)
    .expect("second cycle should settle");
    let second_heat = fixture
        .state
        .enterprises()
        .get_cycle(second)
        .expect("second cycle should exist")
        .investigation_heat();
    let second_record = fixture
        .state
        .enterprises()
        .get_cycle(second)
        .expect("second cycle should exist");
    let first_heat = fixture
        .state
        .enterprises()
        .get_cycle(first)
        .expect("first cycle should exist")
        .investigation_heat();
    assert!(second_record.drew_vice_attention());
    assert_eq!(second_heat, first_heat);
    assert!(second_record.net_cash() >= Money::ZERO);
    assert_eq!(
        second_record.attention(),
        crate::core::attention::AttentionClass::Notable
    );

    // Jurisdiction priority can change while the already-open inquiry remains owned by the
    // original bureau. A new higher-priority bureau taking intake must not make the racket look
    // case-free and recursively open a second simultaneous inquiry on the following settlement.
    let replacement_police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Metropolitan Vice Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("replacement police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: replacement_police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(90),
        },
    )
    .expect("replacement jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("replacement jurisdiction should commit");
    assert_eq!(
        crate::legal::jurisdiction_system::resolve_case_intake_authority(
            &fixture.state,
            neighborhood,
        ),
        Some(replacement_police),
        "replacement bureau must actually own new intake before the handoff regression runs"
    );

    // The inquiry opened above now contributes to district heat itself. Even a guaranteed
    // visibility hit under the replacement intake authority must not duplicate it.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let third = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, 0),
        )
        .expect("third due cycle should resolve"),
    )
    .expect("third cycle plan should validate")
    .commit(&mut fixture.state)
    .expect("third cycle should settle");
    let third_record = fixture
        .state
        .enterprises()
        .get_cycle(third)
        .expect("third cycle should exist");
    let per_case_heat = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .heat_surcharge_per_active_case();
    assert_eq!(
        third_record.investigation_heat(),
        second_heat
            .checked_add(per_case_heat)
            .expect("two bounded authored heat charges should add"),
        "the original bureau's still-active enterprise inquiry must keep contributing heat after intake priority moves elsewhere"
    );
    assert!(
        !third_record.drew_vice_attention(),
        "an active dedicated inquiry must suppress duplicate concurrent inquiries"
    );
    let active_racket_inquiries = fixture
        .state
        .legal()
        .active_investigations()
        .filter(|investigation| {
            investigation.origin() == Some(EntityRef::Enterprise(enterprise))
                && investigation
                    .subjects()
                    .contains(&EntityRef::Enterprise(enterprise))
        })
        .count();
    assert_eq!(active_racket_inquiries, 1);

    // The persisted artifact must satisfy the authored-content validator that every save and
    // load runs — the exact check the missing disjunct used to fail.
    validate_state_against_registry(&registry, &fixture.state)
        .expect("a vice-drawn notable cycle must stay registry-valid");

    // The load boundary must also keep the two information channels causally distinct. These
    // corruptions preserve the exact bincode layout (Some<InformationId> stays Some and the
    // vice flag is a fixed-width bool), so restore reaches structural validation rather than
    // failing merely because the bytes are undecodable.
    let second_record = fixture
        .state
        .enterprises()
        .get_cycle(second)
        .expect("vice cycle should remain available for corruption checks");
    let mut false_vice_flag = enterprise_cycle_wire(second_record);
    false_vice_flag.artifacts.drew_vice_attention = false;
    let error = restore_save(
        &registry,
        replace_serialized_cycle(
            build_save(&registry, &fixture.state)
                .expect("valid state should save before false-vice corruption"),
            second_record,
            &false_vice_flag,
        ),
    )
    .expect_err("a cycle cannot retain vice information while denying the vice event");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidEnterpriseCycle {
                    cycle: invalid,
                }
            ) if invalid == second
        ),
        "expected invalid vice-flag enterprise cycle, got {error:?}"
    );

    let mut wrong_vice_information = enterprise_cycle_wire(second_record);
    wrong_vice_information.provenance.vice_information = second_record.information();
    let error = restore_save(
        &registry,
        replace_serialized_cycle(
            build_save(&registry, &fixture.state)
                .expect("valid state should save before vice-topic corruption"),
            second_record,
            &wrong_vice_information,
        ),
    )
    .expect_err("vice provenance must point to LegalActivity information");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidEnterpriseCycle {
                    cycle: invalid,
                }
            ) if invalid == second
        ),
        "expected invalid vice enterprise cycle, got {error:?}"
    );

    let mut wrong_manager_information = enterprise_cycle_wire(second_record);
    wrong_manager_information.provenance.information = second_record.vice_information();
    let error = restore_save(
        &registry,
        replace_serialized_cycle(
            build_save(&registry, &fixture.state)
                .expect("valid state should save before manager-topic corruption"),
            second_record,
            &wrong_manager_information,
        ),
    )
    .expect_err("manager report provenance must point to FinancialPerformance information");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidEnterpriseCycle {
                    cycle: invalid,
                }
            ) if invalid == second
        ),
        "expected invalid manager-information cycle, got {error:?}"
    );

    let bytes =
        bincode::serialize(&build_save(&registry, &fixture.state).expect("save should build"))
            .expect("save should serialize");
    let restored = restore_save(
        &registry,
        bincode::deserialize::<SaveEnvelope>(&bytes).expect("save should deserialize"),
    )
    .expect("vice-drawn state should restore");
    validate_invariants(&restored);
}

#[test]
fn routine_cycle_records_causal_economics_and_balanced_cash_settlement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("due enterprise cycle should resolve");
    assert_eq!(
        plan.net_cash(),
        plan.gross_revenue()
            .checked_sub(plan.operating_cost())
            .expect("net should be gross - cost")
    );
    assert!(plan.gross_revenue().cents() > 0);
    assert!(plan.operating_cost().cents() > 0);

    let cycle = validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("cycle settlement should commit");
    let cycle_record = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("cycle should exist");
    assert!(cycle_record.transaction().is_some());
    let cash_balance = fixture
        .state
        .finance()
        .get_account(fixture.cash)
        .expect("cash account should exist")
        .balance();
    let settlement_balance = fixture
        .state
        .finance()
        .get_account(fixture.settlement)
        .expect("settlement account should exist")
        .balance();
    assert_eq!(cash_balance, cycle_record.net_cash());
    assert_eq!(settlement_balance, Money::from_cents(-cash_balance.cents()));
    validate_invariants(&fixture.state);
}

#[test]
fn registry_validation_rejects_internally_balanced_unauthored_enterprise_financials() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("due enterprise cycle should resolve");
    let cycle = validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("enterprise cycle should validate")
        .commit(&mut fixture.state)
        .expect("enterprise cycle should commit");

    // Keep net and its ledger transaction unchanged while shifting gross and cost together.
    // Corrupt the persisted wire representation rather than bypassing the enterprise owner:
    // the real load boundary must reject internally balanced economics that authored content
    // could not have produced.
    let delta = Money::from_cents(100);
    let record = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("cycle fixture should persist");
    let mut corrupted = enterprise_cycle_wire(record);
    corrupted.financials.gross_revenue = corrupted
        .financials
        .gross_revenue
        .checked_add(delta)
        .expect("fixture corruption should fit money");
    corrupted.financials.operating_cost = corrupted
        .financials
        .operating_cost
        .checked_add(delta)
        .expect("fixture corruption should fit money");
    let envelope = build_save(&registry, &fixture.state)
        .expect("valid enterprise cycle should save before corruption");
    let corrupted_envelope = replace_serialized_cycle(envelope, record, &corrupted);
    let error = restore_save(&registry, corrupted_envelope)
        .expect_err("unauthored enterprise economics must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidEnterpriseCycle {
                    cycle: invalid,
                }
            ) if invalid == cycle
        ),
        "expected invalid enterprise cycle, got {error:?}"
    );
}

#[test]
fn district_heat_surcharge_scopes_to_the_enterprise_neighborhood() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let local_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(neighborhood) => neighborhood,
        EnterpriseLocation::Business(_) => {
            panic!("enterprise fixture should be located in a neighborhood")
        }
    };
    let other_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Dock Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("other neighborhood fixture should validate");
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Metro Police Authority".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([local_neighborhood, other_neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .expect("spanning jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("spanning jurisdiction should commit");
    let manager = fixture.authority.manager;

    let open_heat_case = |fixture: &mut EnterpriseFixture, title: &str, target| {
        let origin = validate_authorize_operation(
            &registry,
            &fixture.state,
            OperationDraft {
                title: format!("{title} origin patrol"),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.organization,
                leader: manager,
                objective: OperationObjective::GatherInformation { target },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, manager)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("origin operation should validate")
        .commit(&mut fixture.state)
        .expect("origin operation should commit");
        validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: police,
                title: title.to_owned(),
                subjects: BTreeSet::from([EntityRef::Operation(origin)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Operation(origin),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin: Some(EntityRef::Operation(origin)),
                notified_organizations: BTreeSet::from([fixture.organization]),
                witness: None,
            },
        )
        .expect("incident intake should validate")
        .commit(&mut fixture.state)
        .expect("incident intake should commit");
    };
    let due_cycle = |fixture: &mut EnterpriseFixture| {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("due enterprise cycle should resolve");
        let (cost, heat, attention) = (
            plan.operating_cost(),
            plan.investigation_heat(),
            plan.attention(),
        );
        validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("cycle plan should validate")
            .commit(&mut fixture.state)
            .expect("cycle settlement should commit");
        (cost, heat, attention)
    };

    let (baseline_cost, baseline_heat, baseline_attention) = due_cycle(&mut fixture);
    assert_eq!(baseline_heat, Money::ZERO);
    assert_eq!(baseline_attention, AttentionClass::Routine);
    // A case targeting another district of the same authority must not tax this racket.
    open_heat_case(
        &mut fixture,
        "Dock ward inquiry",
        EntityRef::Neighborhood(other_neighborhood),
    );
    let (cross_district_cost, cross_district_heat, _) = due_cycle(&mut fixture);
    assert_eq!(cross_district_cost, baseline_cost);
    assert_eq!(cross_district_heat, Money::ZERO);
    // A case targeting the enterprise's own district raises the daily cost by $50, becomes
    // notable, and records a player-visible report explaining the street surcharge.
    open_heat_case(
        &mut fixture,
        "Market ward inquiry",
        EntityRef::Neighborhood(local_neighborhood),
    );
    let (local_cost, local_heat, local_attention) = due_cycle(&mut fixture);
    assert_eq!(
        local_cost,
        cross_district_cost
            .checked_add(Money::from_cents(5_000))
            .expect("heat surcharge arithmetic should not overflow")
    );
    assert_eq!(local_heat, Money::from_cents(5_000));
    assert_eq!(local_attention, AttentionClass::Notable);
    let hot_cycle = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .max_by_key(|cycle| cycle.occurred_at())
        .expect("hot cycle should persist");
    let hot_information = fixture
        .state
        .intelligence()
        .get_information(
            hot_cycle
                .information()
                .expect("hot cycle must carry its report"),
        )
        .expect("cycle report information must persist");
    assert!(
        hot_information
            .summary()
            .contains("$50.00 street surcharge while police work stays heavy")
    );
    validate_state(&fixture.state).expect("district heat state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn sustained_identical_heat_reports_once_then_routine_until_it_changes() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let local_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(neighborhood) => neighborhood,
        EnterpriseLocation::Business(_) => {
            panic!("enterprise fixture should be located in a neighborhood")
        }
    };
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Sustained Heat Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([local_neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .expect("jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("jurisdiction should commit");
    let manager = fixture.authority.manager;

    let open_case = |fixture: &mut EnterpriseFixture, title: &str| {
        let origin = validate_authorize_operation(
            &registry,
            &fixture.state,
            OperationDraft {
                title: format!("{title} origin patrol"),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.organization,
                leader: manager,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Neighborhood(local_neighborhood),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, manager)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("origin operation should validate")
        .commit(&mut fixture.state)
        .expect("origin operation should commit");
        validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: police,
                title: title.to_owned(),
                subjects: BTreeSet::from([EntityRef::Operation(origin)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Operation(origin),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin: Some(EntityRef::Operation(origin)),
                notified_organizations: BTreeSet::from([fixture.organization]),
                witness: None,
            },
        )
        .expect("incident intake should validate")
        .commit(&mut fixture.state)
        .expect("incident intake should commit");
    };
    let settle_cycle = |fixture: &mut EnterpriseFixture| {
        // Settle one due cycle and return the attention class its plan carried.
        settle_cycle_inner(&registry, fixture, enterprise)
    };

    open_case(&mut fixture, "First ward inquiry");
    let first_hot = settle_cycle(&mut fixture);
    assert_eq!(first_hot, AttentionClass::Notable);

    // The next cycle pays the same surcharge while the case stays open. The cost is known
    // news by now, so it settles as routine instead of repeating an identical report.
    let second_hot = settle_cycle(&mut fixture);
    assert_eq!(second_hot, AttentionClass::Routine);

    open_case(&mut fixture, "Second ward inquiry");
    let escalated = settle_cycle(&mut fixture);
    assert_eq!(escalated, AttentionClass::Notable);

    validate_state(&fixture.state).expect("sustained heat state should validate");
    validate_invariants(&fixture.state);
}

/// Settles one due cycle for `enterprise` and returns its committed attention class.
fn settle_cycle_inner(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    enterprise: EnterpriseId,
) -> AttentionClass {
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("due enterprise cycle should resolve");
    let attention = plan.attention();
    validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("cycle settlement should commit");
    attention
}

#[test]
fn detained_enterprise_manager_pauses_due_cycles_until_release() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let manager = fixture.authority.manager;
    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Enterprise Custody Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: police,
            title: "Enterprise manager custody inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(manager)]),
        },
    )
    .expect("custody investigation should validate")
    .commit(&mut fixture.state)
    .expect("custody investigation should commit");
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(manager),
            origin: None,
            kind: EvidenceKind::FinancialRecord,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("custody evidence should validate")
    .commit(&mut fixture.state)
    .expect("custody evidence should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let arrest = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: manager,
            investigation,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("manager arrest should not require revoking formal enterprise authority")
    .commit(&mut fixture.state)
    .expect("manager arrest should commit");

    assert!(find_due_enterprises(&fixture.state).is_empty());
    let detained_tick = run_tick(&registry, &mut fixture.state);
    assert!(detained_tick.enterprise_cycles.is_empty());
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should persist")
            .next_cycle_at(),
        Some(SimTime::from_minutes(1_440))
    );
    // Stay one minute short of the authored custody maximum: the overdue cycle must remain
    // paused while its manager is still actually detained.
    fixture.state.advance_clock(SimDuration::from_minutes(
        registry.legal().maximum_detention().as_minutes() - 3,
    ));
    let still_detained_tick = run_tick(&registry, &mut fixture.state);
    assert!(still_detained_tick.enterprise_cycles.is_empty());
    assert!(still_detained_tick.custody_releases.is_empty());
    validate_state(&fixture.state).expect("paused enterprise detention state should validate");
    validate_invariants(&fixture.state);

    // The next canonical minute reaches the custody cap. Release runs before economy settlement,
    // so the manager becomes available and the single overdue enterprise cycle settles immediately.
    let released_tick = run_tick(&registry, &mut fixture.state);
    assert_eq!(released_tick.custody_releases, vec![arrest]);
    assert_eq!(released_tick.enterprise_cycles.len(), 1);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_cycle(released_tick.enterprise_cycles[0])
            .expect("released manager should produce the overdue enterprise cycle")
            .enterprise(),
        enterprise
    );
    let next_cycle_at = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should persist after release")
        .next_cycle_at();
    assert_eq!(
        next_cycle_at,
        Some(fixture.state.now() + SimDuration::from_minutes(1_440))
    );
    let no_burst_tick = run_tick(&registry, &mut fixture.state);
    assert!(no_burst_tick.enterprise_cycles.is_empty());
    validate_state(&fixture.state).expect("resumed enterprise state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn settlement_account_is_exclusive_to_one_enterprise_history() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let first = establish_protection(&registry, &mut fixture);

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("settlement account reuse must fail before mutation"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::SettlementAccountInUse {
            account: fixture.settlement,
            enterprise: first,
        }
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_at(fixture.location)
            .count(),
        1
    );
    validate_invariants(&fixture.state);
}

#[test]
fn business_hosted_gambling_requires_concrete_venue_functions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let incomplete_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Sparse Storefront".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("incomplete venue should still be a valid business");

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(incomplete_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("gambling must reject a venue without its required functions"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::MissingBusinessFunction {
            business: incomplete_venue,
            function: BusinessFunction::CashIntensive,
        }
    );

    let valid_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Market Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("complete venue should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(valid_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("gambling should accept a venue with all required functions")
    .commit(&mut fixture.state)
    .expect("business-hosted enterprise should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should exist")
            .location(),
        EnterpriseLocation::Business(valid_venue)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn alcohol_distribution_uses_owned_business_network_and_survives_save_before_cycle() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let enterprise = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail]),
    )
    .expect("complete owned distribution network should establish");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_supported_by_business(transport)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![enterprise]
    );
    validate_state(&fixture.state).expect("alcohol distribution state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("alcohol distribution network should satisfy authored content");
    validate_invariants(&fixture.state);

    let save = build_save(&registry, &fixture.state)
        .expect("alcohol distribution state should build a save");
    let bytes = bincode::serialize(&save).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored = restore_save(&registry, decoded)
        .expect("alcohol distribution support indexes should restore");
    assert_eq!(
        restored
            .enterprises()
            .enterprises_supported_by_business(retail)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![enterprise]
    );

    restored.advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &restored,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("valid alcohol distribution network should resolve a due cycle");
    assert_eq!(
        plan.net_cash(),
        plan.gross_revenue()
            .checked_sub(plan.operating_cost())
            .expect("net should be gross - cost")
    );
    assert!(plan.gross_revenue().cents() > plan.operating_cost().cents());
    validate_enterprise_cycle_plan(&restored, plan)
        .expect("fresh alcohol distribution cycle should validate")
        .commit(&mut restored)
        .expect("alcohol distribution cycle should commit");
    validate_state(&restored).expect("resolved alcohol distribution state should validate");
    validate_state_against_registry(&registry, &restored)
        .expect("resolved alcohol distribution state should remain authored-valid");
    validate_invariants(&restored);
}

#[test]
fn alcohol_distribution_rejects_incomplete_or_foreign_support_networks() {
    let registry = build_registry();
    let mut incomplete = make_test_enterprise_fixture();
    let incomplete_organization = incomplete.organization;
    let transport = insert_support_business(
        &registry,
        &mut incomplete,
        "Incomplete Freight Network",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(incomplete_organization),
    );
    let error =
        establish_alcohol_distribution(&registry, &mut incomplete, BTreeSet::from([transport]))
            .expect_err("distribution network without retail access must be rejected");
    assert_eq!(
        error,
        EnterpriseError::MissingNetworkFunction {
            function: BusinessFunction::CustomerAccess,
        }
    );

    let mut foreign = make_test_enterprise_fixture();
    let network = insert_support_business(
        &registry,
        &mut foreign,
        "Independent Distribution Combine",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Independent,
    );
    let error = establish_alcohol_distribution(&registry, &mut foreign, BTreeSet::from([network]))
        .expect_err("foreign business capacity must not be consumed implicitly");
    assert_eq!(
        error,
        EnterpriseError::SupportingBusinessOwnershipMismatch {
            business: network,
            owner: BusinessOwner::Independent,
            organization: foreign.organization,
        }
    );
    validate_state(&incomplete.state).expect("rejected incomplete network should not mutate");
    validate_state(&foreign.state).expect("rejected foreign network should not mutate");
    validate_invariants(&incomplete.state);
    validate_invariants(&foreign.state);
}

#[test]
fn neighborhood_authority_cannot_bind_support_business_in_another_district() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let local_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood authority"),
    };
    assert_eq!(
        fixture.authority.scope,
        ResponsibilityScope::Neighborhood(local_neighborhood)
    );
    let remote_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Remote Logistics Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(55),
                    illicit_demand: rating(45),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("remote neighborhood should validate");
    let organization = fixture.organization;
    let remote_transport = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Remote Freight Depot".to_owned(),
            kind: BusinessKind::Transportation,
            functions: BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
            ]),
            neighborhood: remote_neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("remote transport should validate");
    let local_retail = insert_support_business(
        &registry,
        &mut fixture,
        "Local Bottle Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );

    let error = match validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([remote_transport, local_retail]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("district authority must not bind remote support infrastructure"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::AuthoritySupportingBusinessMismatch {
            scope: fixture.authority.scope,
            business: remote_transport,
        }
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);
    validate_state(&fixture.state).expect("rejected remote support must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn distribution_establishment_token_stales_when_support_ownership_changes() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let expected_version = fixture
        .state
        .world()
        .get_business(retail)
        .expect("support business should exist")
        .version();
    let establishment = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::AlcoholDistribution,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::from([transport, retail]),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("complete distribution network should initially validate");

    validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
        .expect("no committed enterprise should lock support ownership yet")
        .commit(&mut fixture.state)
        .expect("support ownership transfer should commit before enterprise establishment");
    let found_version = fixture
        .state
        .world()
        .get_business(retail)
        .expect("support business should remain")
        .version();
    assert_eq!(
        establishment
            .commit(&mut fixture.state)
            .expect_err("support mutation must stale validated establishment"),
        EnterpriseError::StaleSupportingBusiness {
            business: retail,
            expected: expected_version,
            found: found_version,
        }
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_supported_by_business(transport)
            .count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .expect("settlement account should persist")
            .balance(),
        Money::ZERO
    );
    validate_state(&fixture.state)
        .expect("stale establishment rejection should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn active_distribution_network_locks_business_ownership_and_resume_revalidates_versions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
    let enterprise = establish_alcohol_distribution(
        &registry,
        &mut fixture,
        BTreeSet::from([transport, retail]),
    )
    .expect("complete network should establish");

    let error =
        validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
            .expect_err("active enterprise must lock supporting business ownership");
    assert_eq!(
        error,
        WorldError::ActiveEnterpriseSupport {
            business: retail,
            enterprise,
            organization: fixture.organization,
        }
    );

    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active distribution enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("distribution suspension should commit");
    let stale_resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("owned support network should initially validate for resume");
    validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
        .expect("suspended enterprise should release support ownership lock")
        .commit(&mut fixture.state)
        .expect("support ownership transfer should commit while suspended");
    assert_eq!(
        stale_resume
            .commit(&mut fixture.state)
            .expect_err("support ownership mutation must stale prior resume token"),
        EnterpriseError::StaleSupportingBusiness {
            business: retail,
            expected: 1,
            found: 2,
        }
    );
    let fresh_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("foreign-owned support network must not resume"),
        Err(error) => error,
    };
    assert_eq!(
        fresh_error,
        EnterpriseError::SupportingBusinessOwnershipMismatch {
            business: retail,
            owner: BusinessOwner::Independent,
            organization: fixture.organization,
        }
    );

    validate_transfer_business_ownership(
        &fixture.state,
        retail,
        BusinessOwner::Organization(fixture.organization),
    )
    .expect("suspended support business should be transferable back")
    .commit(&mut fixture.state)
    .expect("support ownership restoration should commit");
    validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("restored network should resume")
        .commit(&mut fixture.state)
        .expect("restored distribution enterprise resume should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("distribution enterprise should persist")
            .status(),
        EnterpriseStatus::Active
    );
    validate_state(&fixture.state).expect("restored distribution network should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("restored distribution network should satisfy authored content");
    validate_invariants(&fixture.state);
}

#[test]
fn resume_commit_rejects_a_host_venue_that_changed_after_validation() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Resumption Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("host venue fixture should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("business-hosted gambling should establish")
    .commit(&mut fixture.state)
    .expect("business-hosted gambling should commit");
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active venue racket should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    // A resume token validated while the organization still owned the venue must not
    // reactivate the racket after the venue changed hands: the host version pin stales it.
    let stale_resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("owned venue should initially validate for resume");
    validate_transfer_business_ownership(&fixture.state, venue, BusinessOwner::Independent)
        .expect("suspended enterprise should release the host lock")
        .commit(&mut fixture.state)
        .expect("venue sale should commit while suspended");
    assert_eq!(
        stale_resume
            .commit(&mut fixture.state)
            .expect_err("a sold venue must stale the prior resume token"),
        EnterpriseError::StaleHostBusiness {
            business: venue,
            expected: 1,
            found: 2,
        }
    );
    let fresh_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("a foreign-owned venue must not host a resumed racket"),
        Err(error) => error,
    };
    assert_eq!(
        fresh_error,
        EnterpriseError::HostBusinessOwnershipMismatch {
            business: venue,
            owner: BusinessOwner::Independent,
            organization: fixture.organization,
        }
    );
    validate_state(&fixture.state).expect("rejected resume should leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn suspension_removes_enterprise_from_due_work_and_resume_reschedules_it() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(find_due_enterprises(&fixture.state).is_empty());

    let resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .expect("suspended enterprise with valid authority should resume");
    fixture.state.advance_clock(SimDuration::from_minutes(30));
    resume
        .commit(&mut fixture.state)
        .expect("resume should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should exist");
    assert_eq!(record.status(), EnterpriseStatus::Active);
    assert_eq!(
        record.next_cycle_at(),
        Some(fixture.state.now() + SimDuration::from_minutes(1_440))
    );
    validate_invariants(&fixture.state);
}

#[test]
fn enterprise_establishment_schedule_starts_at_commit_time() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let establishment = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("enterprise should validate before delayed commit");
    fixture.state.advance_clock(SimDuration::from_minutes(60));
    let enterprise = establishment
        .commit(&mut fixture.state)
        .expect("delayed enterprise establishment should commit");
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should exist");
    assert_eq!(record.established_at(), SimTime::from_minutes(60));
    assert_eq!(record.next_cycle_at(), Some(SimTime::from_minutes(1_500)));
    validate_invariants(&fixture.state);
}

#[test]
fn stale_cycle_plan_cannot_commit_after_enterprise_lifecycle_change() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("cycle should resolve");
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    let error = match validate_enterprise_cycle_plan(&fixture.state, plan) {
        Ok(_) => panic!("cycle plan must become stale after lifecycle mutation"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::StaleEnterprise {
            enterprise,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should exist")
            .balance(),
        Money::ZERO
    );
    validate_invariants(&fixture.state);
}

#[test]
fn active_enterprise_blocks_authority_removal_until_suspended() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let mandate = fixture.authority.mandate;
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let historical_cycle = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("pre-suspension cycle should decide"),
    )
    .expect("pre-suspension cycle should validate")
    .commit(&mut fixture.state)
    .expect("pre-suspension cycle should commit");

    let revoke_error = validate_revoke_mandate(&fixture.state, mandate)
        .expect_err("active routine work must block mandate revocation");
    assert_eq!(
        revoke_error,
        DelegationError::ActiveEnterpriseDependency {
            mandate,
            enterprise,
        }
    );

    let replacement_scope = ResponsibilityScope::Function(ResponsibilityFunction::Finance);
    let revision_error = validate_revise_mandate(
        &fixture.state,
        mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([replacement_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect_err("active routine work must preserve its delegated scope");
    assert_eq!(
        revision_error,
        DelegationError::ActiveEnterpriseScopeDependency {
            mandate,
            enterprise,
            scope: fixture.authority.scope,
        }
    );

    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("enterprise should suspend before authority is removed")
        .commit(&mut fixture.state)
        .expect("enterprise suspension should commit");
    validate_revoke_mandate(&fixture.state, mandate)
        .expect("suspended routine work should release active mandate dependency")
        .commit(&mut fixture.state)
        .expect("mandate revocation should commit after suspension");

    let resume_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("enterprise must not resume under revoked authority"),
        Err(error) => error,
    };
    assert_eq!(
        resume_error,
        EnterpriseError::Delegation(DelegationError::InactiveMandate(mandate))
    );

    let later_organization = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Former Manager Employer".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("later organization should validate");
    validate_reassign_character(
        &fixture.state,
        fixture.authority.manager,
        Some(later_organization),
        None,
    )
    .expect("suspended enterprise and revoked mandate should release the former manager")
    .commit(&mut fixture.state)
    .expect("former enterprise manager should transfer");

    validate_state(&fixture.state)
        .expect("suspended enterprise history must tolerate later manager membership");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("historical enterprise economics must remain valid after manager transfer");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("suspended enterprise history should save after manager transfer"),
    )
    .expect("suspended enterprise history should restore after manager transfer");
    assert!(restored.enterprises().get_cycle(historical_cycle).is_some());
    assert_eq!(
        restored
            .world()
            .get_character(fixture.authority.manager)
            .expect("restored former manager should persist")
            .organization(),
        Some(later_organization)
    );
    validate_invariants(&restored);
}

#[test]
fn save_round_trip_preserves_due_schedule_and_deterministic_cycle_resolution() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    let envelope = build_save(&registry, &fixture.state)
        .expect("active enterprise state should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored =
        restore_save(&registry, decoded).expect("enterprise save should restore cleanly");
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .expect("restored enterprise should exist")
            .next_cycle_at(),
        Some(SimTime::from_minutes(1_440))
    );

    let original_outcome = run_tick(&registry, &mut fixture.state);
    let restored_outcome = run_tick(&registry, &mut restored);
    assert_eq!(original_outcome, restored_outcome);
    assert_eq!(original_outcome.enterprise_cycles.len(), 1);
    let cycle = original_outcome.enterprise_cycles[0];
    let original_cycle = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("original cycle should exist");
    let restored_cycle = restored
        .enterprises()
        .get_cycle(cycle)
        .expect("restored continuation should create the same cycle ID");
    assert_eq!(
        original_cycle.gross_revenue(),
        restored_cycle.gross_revenue()
    );
    assert_eq!(
        original_cycle.operating_cost(),
        restored_cycle.operating_cost()
    );
    assert_eq!(original_cycle.net_cash(), restored_cycle.net_cash());
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("original cash account should exist")
            .balance(),
        restored
            .finance()
            .get_account(fixture.cash)
            .expect("restored cash account should exist")
            .balance()
    );
    validate_invariants(&fixture.state);
    validate_invariants(&restored);
}

#[test]
fn financial_reporting_drills_down_without_cached_totals() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    for variance in [0, 700] {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(variance, u16::MAX),
        )
        .expect("due cycle should resolve for reporting fixture");
        validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("reporting fixture cycle should validate")
            .commit(&mut fixture.state)
            .expect("reporting fixture cycle should commit");
    }

    let period_start = SimTime::ZERO;
    let period_end = fixture.state.now();
    let enterprise_summary =
        resolve_enterprise_financial_summary(&fixture.state, enterprise, period_start, period_end)
            .expect("enterprise financial summary should resolve");
    let organization_summary = resolve_organization_enterprise_financial_summary(
        &fixture.state,
        fixture.organization,
        period_start,
        period_end,
    )
    .expect("organization financial summary should resolve");
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let neighborhood_summary = resolve_neighborhood_enterprise_financial_summary(
        &fixture.state,
        neighborhood,
        period_start,
        period_end,
    )
    .expect("neighborhood financial summary should resolve");

    assert_eq!(enterprise_summary.totals.enterprise_count, 1);
    assert_eq!(enterprise_summary.totals.cycle_count, 2);
    assert_eq!(enterprise_summary.totals.notable_cycle_count, 1);
    assert_eq!(enterprise_summary.totals, organization_summary.totals);
    assert_eq!(enterprise_summary.totals, neighborhood_summary.totals);
    assert_eq!(
        enterprise_summary
            .by_kind
            .get(&EnterpriseKind::Protection)
            .expect("protection bucket should exist"),
        &enterprise_summary.totals
    );
    let cycle_net = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .try_fold(Money::ZERO, |total, cycle| {
            total.checked_add(cycle.net_cash())
        })
        .expect("reporting fixture total should not overflow");
    assert_eq!(enterprise_summary.totals.net_cash, cycle_net);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should exist")
            .balance(),
        enterprise_summary.totals.net_cash
    );
    validate_invariants(&fixture.state);
}

fn designate_player(registry: &Registry, state: &mut AppState) -> OrganizationId {
    let player = insert_organization(
        registry,
        state,
        OrganizationDraft {
            name: "Player Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("player organization fixture should validate");
    crate::world::world_system::designate_player_organization(state, player)
        .expect("player designation fixture should validate");
    player
}

#[test]
fn autonomous_expansion_serves_governed_rivals_and_never_the_player_organization() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    let player = designate_player(&registry, &mut fixture.state);

    // The rival's mandate covers the district; the player organization has no mandate at all,
    // so even the designated player cannot receive autonomous establishments here.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "exactly one governed rival establishment per pass"
    );
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous enterprise should persist");
    assert_eq!(record.organization(), fixture.organization);
    assert_eq!(record.kind(), EnterpriseKind::Protection);
    assert_eq!(record.location(), fixture.location);
    // Asset-free kinds settle at the district itself through the covering scope.
    assert!(!matches!(
        record.location(),
        EnterpriseLocation::Business(_)
    ));
    validate_invariants(&fixture.state);
    let _ = player;
}

#[test]
fn autonomous_expansion_is_a_daily_cadence_gate() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("off-cadence autonomous expansion should resolve")
            .is_empty()
    );
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("due autonomous expansion should resolve")
            .len(),
        1,
        "the pass fires exactly on the day boundary"
    );
}

#[test]
fn autonomous_expansion_requires_one_current_cycle_of_working_capital() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let required = resolve_current_enterprise_operating_cost(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
    )
    .expect("fixture enterprise operating cost should fit");
    assert!(required > Money::ZERO);
    fund_enterprise_fixture_cash(&mut fixture, required.cents() - 1);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("undercapitalized autonomous expansion should resolve without mutation");
    assert!(
        established.is_empty(),
        "delegated expansion needs one current operating cycle of working capital"
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);

    fund_enterprise_fixture_cash(&mut fixture, 1);
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("fully capitalized autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_skips_unaffordable_kind_for_later_affordable_kind() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let protection = establish_protection(&registry, &mut fixture);
    let protection_runway = resolve_current_enterprise_operating_cost(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
    )
    .expect("existing protection runway should fit");
    fund_enterprise_fixture_cash(
        &mut fixture,
        protection_runway
            .cents()
            .checked_add(6_200)
            .expect("fixture funding should fit"),
    );

    // This venue makes Gambling a valid earlier candidate, but its current-cycle runway is
    // above the $62 uncommitted treasury after reserving the existing Protection runway.
    // Bookmaking remains affordable with the same venue, so financing-aware selection must
    // continue instead of abandoning the pass.
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Lean Rival Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("financing-aware autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(established[0])
            .expect("selected enterprise should persist")
            .kind(),
        EnterpriseKind::Bookmaking
    );
    assert!(
        fixture
            .state
            .enterprises()
            .get_enterprise(protection)
            .is_some()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_does_not_spend_existing_racket_runway_twice() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 6_200);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Runway Guard Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("working-capital reservation pass should resolve");
    assert!(
        established.is_empty(),
        "cash already backing an active racket cannot simultaneously fund another runway"
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 1);
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_reserves_new_runway_across_same_day_mandates() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let first_neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
    };
    let one_runway = resolve_current_enterprise_operating_cost(
        &registry,
        &fixture.state,
        EnterpriseKind::Protection,
        fixture.location,
        0,
    )
    .expect("protection runway should fit");
    fund_enterprise_fixture_cash(
        &mut fixture,
        one_runway
            .cents()
            .checked_mul(2)
            .and_then(|cents| cents.checked_sub(1))
            .expect("fixture funding should fit"),
    );

    let second_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Second Governed Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("second neighborhood should validate");
    let second_manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second Enterprise Manager".to_owned(),
            organization: Some(fixture.organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second manager should validate");
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager: second_manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(second_neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("second mandate should validate")
    .commit(&mut fixture.state)
    .expect("second mandate should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("multi-mandate autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "one cash pool that cannot cover two runways must not create two same-day rackets"
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(established[0])
            .expect("first establishment should persist")
            .location(),
        EnterpriseLocation::Neighborhood(first_neighborhood),
        "stable mandate order should award the scarce runway to the earlier mandate"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_assembles_authored_multi_business_network() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);

    // Alcohol distribution is authored as a district racket backed by a network, not a venue:
    // transport/storage/distribution can come from one owned business while customer access
    // comes from another. The autonomous planner must compose the same support shape accepted by
    // the canonical establishment path instead of looking for an impossible all-in-one host.
    let organization = fixture.organization;
    let transport = insert_support_business(
        &registry,
        &mut fixture,
        "Autonomous Freight Network",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let retail = insert_support_business(
        &registry,
        &mut fixture,
        "Autonomous Bottle Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("complete authored network should support autonomous expansion");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(enterprise.location(), fixture.location);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([transport, retail])
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_prefers_support_that_covers_more_unmet_network_functions() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Insert partial businesses first so raw ID order alone would choose redundant support.
    // A later integrated depot covers every authored AlcoholDistribution network function and
    // should therefore be selected alone, avoiding the per-support operating surcharge.
    insert_support_business(
        &registry,
        &mut fixture,
        "Partial Freight Yard",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
        ]),
        BusinessOwner::Organization(organization),
    );
    insert_support_business(
        &registry,
        &mut fixture,
        "Partial Retail Counter",
        BusinessKind::Retail,
        BTreeSet::from([BusinessFunction::CustomerAccess]),
        BusinessOwner::Organization(organization),
    );
    let integrated = insert_support_business(
        &registry,
        &mut fixture,
        "Integrated Distribution Depot",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("integrated support network should support autonomous expansion");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([integrated]),
        "support planning should not attach redundant surcharge-producing businesses"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_finds_minimum_support_cover_when_greedy_would_overpay() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Required AlcoholDistribution network functions are vehicle, warehouse, distribution,
    // customer access. A greedy ID-first maximum-coverage choice would take `first` ({V,W}),
    // then need both later businesses. The exact cover is the latter two businesses only.
    let first = insert_support_business(
        &registry,
        &mut fixture,
        "Greedy Trap Storage",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
        ]),
        BusinessOwner::Organization(organization),
    );
    let distribution = insert_support_business(
        &registry,
        &mut fixture,
        "Distribution Link",
        BusinessKind::Transportation,
        BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let retail = insert_support_business(
        &registry,
        &mut fixture,
        "Warehouse Retail Link",
        BusinessKind::Retail,
        BTreeSet::from([
            BusinessFunction::Warehousing,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("minimum-cover autonomous expansion should resolve");
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous distribution enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::AlcoholDistribution);
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([distribution, retail])
    );
    assert!(!enterprise.supporting_businesses().contains(&first));
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_chooses_cheaper_host_within_same_district_authority() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    establish_protection(&registry, &mut fixture);
    let organization = fixture.organization;

    // Both clubs satisfy the Speakeasy venue requirements. The lower-ID club contributes none
    // of the supply-chain network, so it would need both the higher-ID club and the brewery as
    // supports. The higher-ID club contributes distribution itself and needs only the brewery.
    // District authority is identical, therefore the manager should avoid the extra $75/cycle
    // authored support surcharge instead of blindly taking the first business ID.
    let _expensive_host = insert_support_business(
        &registry,
        &mut fixture,
        "Early Nightlife Club",
        BusinessKind::Nightclub,
        BTreeSet::from([
            BusinessFunction::Nightlife,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let cheaper_host = insert_support_business(
        &registry,
        &mut fixture,
        "Integrated Nightlife Club",
        BusinessKind::Nightclub,
        BTreeSet::from([
            BusinessFunction::Nightlife,
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ]),
        BusinessOwner::Organization(organization),
    );
    let brewery = insert_support_business(
        &registry,
        &mut fixture,
        "Supply Brewery",
        BusinessKind::Brewery,
        BTreeSet::from([BusinessFunction::AlcoholProduction]),
        BusinessOwner::Organization(organization),
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("cheaper hosted expansion should resolve");
    assert_eq!(established.len(), 1);
    let enterprise = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("autonomous hosted enterprise should persist");
    assert_eq!(enterprise.kind(), EnterpriseKind::Speakeasy);
    assert_eq!(
        enterprise.location(),
        EnterpriseLocation::Business(cheaper_host)
    );
    assert_eq!(
        enterprise.supporting_businesses(),
        &BTreeSet::from([brewery])
    );
    validate_invariants(&fixture.state);
}

#[test]
fn payroll_can_use_organization_cash_referenced_by_an_enterprise() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let outcome =
        crate::world::payroll_execution::apply_daily_payroll(&registry, &mut fixture.state)
            .expect("payroll should settle from organization-owned liquid cash")
            .into_iter()
            .find(|outcome| outcome.organization() == fixture.organization)
            .expect("the staffed organization should run payroll");
    assert_eq!(outcome.paid(), outcome.owed());
    assert_eq!(outcome.short(), Money::ZERO);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("enterprise cash account should persist")
            .balance(),
        Money::from_cents(10_000)
            .checked_sub(outcome.owed())
            .expect("funded fixture can cover one member's payroll")
    );
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_surfaces_enterprise_id_exhaustion_without_partial_establishment() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let enterprise_count_before = fixture.state.enterprises().enterprises().count();
    let account_count_before = fixture.state.finance().accounts().count();
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Enterprise, u32::MAX);

    let error = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect_err("autonomous expansion must surface enterprise allocator exhaustion");
    assert!(matches!(
        error,
        crate::enterprises::autonomous_expansion::AutonomousExpansionError::Enterprise(
            EnterpriseError::IdExhaustion(_)
        )
    ));
    assert_eq!(
        fixture.state.enterprises().enterprises().count(),
        enterprise_count_before,
        "failed expansion must not insert an enterprise"
    );
    assert_eq!(
        fixture.state.finance().accounts().count(),
        account_count_before,
        "failed expansion must not consume or open finance records"
    );
    validate_state(&fixture.state).expect("failed autonomous expansion must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_expansion_rotates_kinds_and_hosts_the_rival_venue() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 20_000);

    // An owned hospitality venue inside the governed district can host every
    // cash-and-space racket kind.
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Rival Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-one autonomous expansion should resolve");
    assert_eq!(first_day.len(), 1);
    let first_kind = {
        let first = fixture
            .state
            .enterprises()
            .get_enterprise(first_day[0])
            .expect("day-one enterprise should persist");
        (first.kind(), first.location(), first.settlement_account())
    };
    assert_eq!(
        first_kind.0,
        EnterpriseKind::Protection,
        "authored kind order puts asset-free rackets first"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-two autonomous expansion should resolve");
    assert_eq!(second_day.len(), 1);
    // Asset-free rackets occupy district scope only. The next day therefore advances to the
    // first venue-backed kind instead of duplicating Protection at the card room.
    let second_kind = {
        let second_probe = fixture
            .state
            .enterprises()
            .get_enterprise(second_day[0])
            .expect("day-two enterprise should persist");
        (second_probe.kind(), second_probe.location())
    };
    assert_eq!(second_kind.0, EnterpriseKind::Gambling);
    assert!(matches!(second_kind.1, EnterpriseLocation::Business(_)));
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let third_day = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("day-three autonomous expansion should resolve");
    assert_eq!(third_day.len(), 1);
    let third = fixture
        .state
        .enterprises()
        .get_enterprise(third_day[0])
        .expect("day-three enterprise should persist");
    assert_eq!(third.kind(), EnterpriseKind::Bookmaking);
    let EnterpriseLocation::Business(host) = third.location() else {
        panic!(
            "bookmaking must host at the venue, got {:?}",
            third.location()
        );
    };
    let host_name = fixture
        .state
        .world()
        .get_business(host)
        .expect("hosted venue should exist")
        .name()
        .to_owned();
    assert_eq!(host_name, "Rival Card Room");

    // Each establishment reserved its own exclusive settlement account.
    assert_ne!(first_kind.2, third.settlement_account());
    validate_invariants(&fixture.state);
}

#[test]
fn same_tick_vice_fear_blocks_due_autonomous_expansion() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 1_000_000);
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let organization = fixture.organization;
    insert_support_business(
        &registry,
        &mut fixture,
        "Same-Tick Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Same-Tick Vice Bureau",
        neighborhood,
    );

    // Enough independent district-pressure cases make this enterprise's due vice roll certain.
    // They deliberately target the neighborhood rather than the enterprise so they create heat
    // without already counting as the dedicated inquiry the cycle should draw.
    let per_case = u32::from(
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .vice_attention_basis_points_per_active_case(),
    );
    assert!(
        per_case > 0,
        "the authored protection racket must carry vice risk"
    );
    let pressure_case_count = 10_000_u32.div_ceil(per_case);
    for index in 0..pressure_case_count {
        validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: police,
                title: format!("Same-tick district pressure {index}"),
                subjects: BTreeSet::from([EntityRef::Neighborhood(neighborhood)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Neighborhood(neighborhood),
                    origin: Some(EntityRef::Enterprise(enterprise)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin: Some(EntityRef::Enterprise(enterprise)),
                notified_organizations: BTreeSet::from([fixture.organization]),
                witness: None,
            },
        )
        .expect("district-pressure intake should validate")
        .commit(&mut fixture.state)
        .expect("district-pressure intake should commit");
    }

    // Start one point below the value that will become the ceiling after day-boundary decay
    // followed by the authored vice consequence: 45 -> 44 decay -> +6 vice = 50.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        5,
    )
    .expect("pre-tick fear setup should apply");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    // Prove the organization really would expand at this boundary if it read the stale
    // pre-consequence posture. This control clone deliberately invokes only the expansion pass.
    let mut stale_posture_control = fixture.state.clone();
    stale_posture_control.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(
        apply_due_autonomous_enterprises(&registry, &mut stale_posture_control)
            .expect("pre-consequence posture should support expansion")
            .len(),
        1,
        "the regression requires a genuinely eligible expansion under the old posture"
    );

    let outcome = run_tick(&registry, &mut fixture.state);
    assert_eq!(outcome.enterprise_cycles.len(), 1);
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(outcome.enterprise_cycles[0])
            .expect("due enterprise cycle should persist")
            .drew_vice_attention(),
        "certainty-level district pressure must draw the same-tick vice inquiry"
    );
    assert_eq!(
        crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        ),
        registry.reputation().expansion_police_fear_ceiling(),
        "day-boundary decay plus the fresh vice consequence should land exactly on the ceiling"
    );
    assert!(
        outcome.autonomous_enterprises.is_empty(),
        "same-minute vice fear must reach the expansion gate before delegated growth runs"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn police_fear_at_or_above_the_authored_ceiling_stalls_expansion_until_it_cools() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 10_000);
    let ceiling = registry.reputation().expansion_police_fear_ceiling();

    // Drive the outfit visibly hot through the canonical reputation path.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        100,
    )
    .expect("fear adjustment should apply");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("hot-posture autonomous expansion should resolve")
            .is_empty(),
        "an outfit at or above the fear ceiling must keep its head down"
    );

    // Exactly the authored ceiling is still hot, not the first safe score.
    loop {
        let fear = crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        );
        if fear <= ceiling {
            break;
        }
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        crate::reputation::reputation_system::apply_daily_reputation_decay(
            &registry,
            &mut fixture.state,
        );
    }
    assert_eq!(
        crate::reputation::reputation_system::resolve_score(
            &registry,
            &fixture.state.reputation,
            fixture.organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        ),
        ceiling
    );
    assert!(
        apply_due_autonomous_enterprises(&registry, &mut fixture.state)
            .expect("ceiling-posture autonomous expansion should resolve")
            .is_empty(),
        "the authored ceiling itself must keep delegated expansion paused"
    );

    // Once the impression decays below the ceiling the same mandate expands again.
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut fixture.state,
        fixture.organization,
        crate::reputation::AudienceKind::Police,
        crate::reputation::ReputationDimension::Fear,
        -1,
    )
    .expect("cooling adjustment should apply");
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(
        established.len(),
        1,
        "cooled-down outfits resume governed expansion"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn expansion_consolidates_led_districts_before_contested_ones() {
    use crate::enterprises::EnterpriseDraft;

    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    // Capital is deliberately non-binding in this influence-ordering test. Existing active
    // rackets reserve their own current-cycle runway before delegated expansion considers a new
    // one, so a token balance would make this a financing test instead of an ordering test.
    fund_enterprise_fixture_cash(&mut fixture, 100_000);

    // Fixture intent: the LED district carries the HIGHER id. Selection that followed raw
    // id order would open in the un-led district first; influence-aware preference must
    // consolidate the led one instead.
    let contested = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use district locations"),
    };
    let led = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Led Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(55),
                    illicit_demand: rating(45),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("led neighborhood should validate");
    assert!(led > contested);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Neighborhood(contested),
                ResponsibilityScope::Neighborhood(led),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate revision should validate")
    .commit(&mut fixture.state)
    .expect("mandate revision should commit");

    // Leadership of the higher-id district: an owned venue hosting a gambling racket.
    let organization = fixture.organization;
    let led_venue = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Led Ward Card Room".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood: led,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("led venue should validate");
    let settlement = crate::finance::finance_system::insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: MandateAuthority {
                mandate: fixture.authority.mandate,
                manager: fixture.authority.manager,
                scope: ResponsibilityScope::Neighborhood(led),
            },
            location: EnterpriseLocation::Business(led_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: settlement,
        },
    )
    .expect("leadership enterprise should validate")
    .commit(&mut fixture.state)
    .expect("leadership enterprise should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("autonomous expansion should resolve");
    assert_eq!(established.len(), 1);
    let location = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("establishment should persist")
        .location();
    assert_eq!(
        location,
        EnterpriseLocation::Neighborhood(led),
        "consolidation preference must outrank raw district id order"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn chronic_losing_enterprise_reports_losses_then_suspends_at_the_authored_threshold() {
    let registry = build_registry();
    let mut state = AppState::new(0xBEEF_5105);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Bleeding Outfit".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Dead Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(5),
                    commercial_activity: rating(5),
                    illicit_demand: rating(5),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(95),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    // A manager with no Management capability earns no management revenue premium.
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Unskilled Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager fixture should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate fixture should validate")
    .commit(&mut state)
    .expect("mandate fixture should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("cash account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    let enterprise = crate::enterprises::enterprise_execution::validate_establish_enterprise(
        &registry,
        &state,
        crate::enterprises::EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization,
            authority: MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .expect("enterprise should establish")
    .commit(&mut state)
    .expect("enterprise should commit");

    let threshold = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .losing_cycles_before_suspension() as usize;
    let mut last_information = None;
    for cycle_index in 0..threshold {
        state.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("losing enterprise cycle should decide");
        assert!(
            plan.net_cash().cents() < 0,
            "fixture must produce a losing settlement"
        );
        assert_eq!(plan.attention(), AttentionClass::Notable);
        let cycle = validate_enterprise_cycle_plan(&state, plan)
            .expect("losing cycle plan should validate")
            .commit(&mut state)
            .expect("losing cycle should commit");
        let record = state
            .enterprises()
            .get_cycle(cycle)
            .expect("cycle record should persist");
        last_information = record.information();
        if cycle_index + 1 < threshold {
            assert_eq!(
                state
                    .enterprises()
                    .get_enterprise(enterprise)
                    .map(|record| record.status()),
                Some(crate::enterprises::EnterpriseStatus::Active),
                "racket stays active below the suspension threshold"
            );
        }
    }
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should persist");
    assert_eq!(
        record.status(),
        crate::enterprises::EnterpriseStatus::Suspended
    );
    assert_eq!(record.next_cycle_at(), None);
    let information = state
        .intelligence()
        .get_information(last_information.expect("notable losing cycle should report"))
        .expect("manager report should persist");
    assert!(information.summary().contains("suspended"));

    crate::enterprises::enterprise_execution::validate_resume_enterprise(
        &registry, &state, enterprise,
    )
    .expect("suspended racket should resume")
    .commit(&mut state)
    .expect("resumed racket should commit");
    assert_eq!(
        state
            .enterprises()
            .get_enterprise(enterprise)
            .map(|record| record.status()),
        Some(crate::enterprises::EnterpriseStatus::Active)
    );

    // Resumption restarts the losing-cycle grace window: pre-suspension losses must not
    // re-suspend the racket on its first post-resume losing settlement.
    for cycle_index in 0..threshold {
        state.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("post-resume losing cycle should decide");
        assert!(plan.net_cash().cents() < 0);
        validate_enterprise_cycle_plan(&state, plan)
            .expect("post-resume losing plan should validate")
            .commit(&mut state)
            .expect("post-resume losing cycle should commit");
        let status = state
            .enterprises()
            .get_enterprise(enterprise)
            .map(|record| record.status());
        if cycle_index + 1 < threshold {
            assert_eq!(
                status,
                Some(crate::enterprises::EnterpriseStatus::Active),
                "a resumed racket gets a fresh grace window"
            );
        } else {
            assert_eq!(
                status,
                Some(crate::enterprises::EnterpriseStatus::Suspended),
                "threshold consecutive post-resume losses suspend again"
            );
        }
    }
    crate::core::invariants::validate_invariants(&state);
}

#[test]
fn establishment_rejects_a_duplicate_kind_at_an_occupied_location_even_when_suspended() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let first = establish_protection(&registry, &mut fixture);
    let duplicate_draft = || EnterpriseDraft {
        kind: EnterpriseKind::Protection,
        organization: fixture.organization,
        authority: fixture.authority,
        location: fixture.location,
        supporting_businesses: BTreeSet::new(),
        cash_account: fixture.cash,
        settlement_account: fixture.settlement,
    };
    assert!(matches!(
        validate_establish_enterprise(&registry, &fixture.state, duplicate_draft()),
        Err(EnterpriseError::DuplicateEnterpriseAtLocation { .. })
    ));
    validate_suspend_enterprise(&fixture.state, first)
        .expect("active enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("enterprise suspension should commit");
    // A suspended racket still occupies its spot until manually resumed; a fresh identical
    // racket must not resurrect the losses the chronic-loss threshold already shut down.
    assert!(matches!(
        validate_establish_enterprise(&registry, &fixture.state, duplicate_draft()),
        Err(EnterpriseError::DuplicateEnterpriseAtLocation { .. })
    ));
    crate::enterprises::enterprise_execution::validate_retire_enterprise(&fixture.state, first)
        .expect("suspended enterprise should be permanently retireable")
        .commit(&mut fixture.state)
        .expect("enterprise retirement should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(first)
            .expect("retired enterprise remains historical")
            .status(),
        EnterpriseStatus::Retired
    );
    let replacement_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("replacement settlement account should validate");
    let replacement_draft = EnterpriseDraft {
        settlement_account: replacement_settlement,
        ..duplicate_draft()
    };
    let replacement = validate_establish_enterprise(&registry, &fixture.state, replacement_draft)
        .expect("retired history must release the kind/location slot while retaining old ledger provenance")
        .commit(&mut fixture.state)
        .expect("replacement enterprise should commit through the normal establishment path");
    assert_ne!(replacement, first);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_at(fixture.location)
            .count(),
        2,
        "retired history and the fresh active replacement both remain queryable"
    );
    crate::core::invariants::validate_invariants(&fixture.state);
}

#[test]
fn retirement_is_terminal_and_requires_prior_suspension() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let active_retire_error =
        match crate::enterprises::enterprise_execution::validate_retire_enterprise(
            &fixture.state,
            enterprise,
        ) {
            Ok(_) => panic!("an active racket must be suspended before permanent retirement"),
            Err(error) => error,
        };
    assert_eq!(
        active_retire_error,
        EnterpriseError::EnterpriseNotSuspended(enterprise)
    );
    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active racket should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");
    crate::enterprises::enterprise_execution::validate_retire_enterprise(
        &fixture.state,
        enterprise,
    )
    .expect("suspended racket should retire")
    .commit(&mut fixture.state)
    .expect("retirement should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("retired history should persist")
            .status(),
        EnterpriseStatus::Retired
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("retired history should persist")
            .next_cycle_at(),
        None
    );
    let resume_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
        Ok(_) => panic!("retirement is terminal"),
        Err(error) => error,
    };
    assert_eq!(resume_error, EnterpriseError::EnterpriseRetired(enterprise));
    let repeated_retire_error =
        match crate::enterprises::enterprise_execution::validate_retire_enterprise(
            &fixture.state,
            enterprise,
        ) {
            Ok(_) => panic!("retirement cannot be repeated"),
            Err(error) => error,
        };
    assert_eq!(
        repeated_retire_error,
        EnterpriseError::EnterpriseRetired(enterprise)
    );

    let save = build_save(&registry, &fixture.state).expect("retired enterprise should save");
    let restored = restore_save(&registry, save).expect("retired enterprise should restore");
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .expect("retired enterprise should survive restore")
            .status(),
        EnterpriseStatus::Retired
    );
    validate_state_against_registry(&registry, &restored)
        .expect("retired enterprise history should remain registry-valid");
    crate::core::invariants::validate_invariants(&restored);
}

#[test]
fn establishment_commit_rejects_a_host_venue_that_changed_after_validation() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let organization = fixture.organization;
    let venue = insert_support_business(
        &registry,
        &mut fixture,
        "Rival Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let validated = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization: fixture.organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("hosted racket should validate against the owned venue");
    // The venue changes hands between validation and commit; the pinned host version must
    // reject the stale plan instead of establishing an unqualified hosting arrangement.
    validate_transfer_business_ownership(&fixture.state, venue, BusinessOwner::Independent)
        .expect("venue transfer should validate")
        .commit(&mut fixture.state)
        .expect("venue transfer should commit");
    assert!(matches!(
        validated.commit(&mut fixture.state),
        Err(EnterpriseError::StaleHostBusiness { .. })
    ));
    assert_eq!(
        fixture
            .state
            .enterprises()
            .enterprises_at(EnterpriseLocation::Business(venue))
            .count(),
        0,
        "a rejected establishment leaves no record behind"
    );
    crate::core::invariants::validate_invariants(&fixture.state);
}

fn insert_district_police(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    name: &str,
    neighborhood: crate::core::id::NeighborhoodId,
) -> OrganizationId {
    let police = insert_organization(
        registry,
        &mut fixture.state,
        OrganizationDraft {
            name: name.to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .expect("jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("jurisdiction should commit");
    police
}

/// Opens an originated district case through canonical intake so the neighborhood carries
/// active case pressure for the vice-attention rolls.
fn open_district_pressure_case(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    police: OrganizationId,
    title: &str,
    neighborhood: crate::core::id::NeighborhoodId,
) {
    let manager = fixture.authority.manager;
    let origin = validate_authorize_operation(
        registry,
        &fixture.state,
        OperationDraft {
            title: format!("{title} origin patrol"),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.organization,
            leader: manager,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Neighborhood(neighborhood),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, manager)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("origin operation should validate")
    .commit(&mut fixture.state)
    .expect("origin operation should commit");
    validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: police,
            title: title.to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(origin)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Operation(origin),
                origin: Some(EntityRef::Operation(origin)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(origin)),
            notified_organizations: BTreeSet::from([fixture.organization]),
            witness: None,
        },
    )
    .expect("pressure case intake should validate")
    .commit(&mut fixture.state)
    .expect("pressure case intake should commit");
}

fn find_vice_investigation(
    fixture: &EnterpriseFixture,
    police: OrganizationId,
    enterprise: EnterpriseId,
) -> Option<crate::legal::InvestigationRecord> {
    fixture
        .state
        .legal
        .investigations_for_owner(police)
        .find(|investigation| investigation.origin() == Some(EntityRef::Enterprise(enterprise)))
        .cloned()
}

#[test]
fn sustained_district_heat_draws_a_vice_inquiry_onto_the_racket_itself() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(&registry, &mut fixture, "Vice Ward Police", neighborhood);

    // A clean district never draws attention even on the strongest visibility roll.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let clean_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("clean cycle plan should resolve");
    assert_eq!(clean_plan.attention(), AttentionClass::Routine);
    validate_enterprise_cycle_plan(&fixture.state, clean_plan)
        .expect("clean cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("clean cycle settlement should commit");
    assert!(
        find_vice_investigation(&fixture, police, enterprise).is_none(),
        "a clean district must not fabricate a vice inquiry"
    );

    // Once originated casework targets the racket's district, a low visibility roll opens a
    // vice inquiry onto the enterprise itself.
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Market ward heat",
        neighborhood,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let hot_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("hot cycle plan should resolve");
    assert_eq!(hot_plan.attention(), AttentionClass::Notable);
    validate_enterprise_cycle_plan(&fixture.state, hot_plan)
        .expect("hot cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("hot cycle settlement should commit");

    let vice_case = find_vice_investigation(&fixture, police, enterprise)
        .expect("the visibility roll must open a vice inquiry on the racket");
    assert_eq!(
        vice_case.status(),
        crate::legal::InvestigationStatus::Active
    );
    assert!(
        vice_case
            .subjects()
            .contains(&EntityRef::Enterprise(enterprise)),
        "the inquiry targets the racket itself"
    );
    assert_eq!(
        vice_case.notified_organizations(),
        &BTreeSet::from([fixture.organization]),
        "the owning organization is surfaced the case-open knowledge"
    );

    // The organization holds provenance-bearing legal knowledge about the inquiry.
    let legal_knowledge = fixture
        .state
        .intelligence()
        .information_for_holder(KnowledgeHolder::Organization(fixture.organization))
        .find(|information| {
            information.topic() == crate::intelligence::InformationTopic::LegalActivity
                && information.subject() == EntityRef::Enterprise(enterprise)
        })
        .expect("vice inquiry must surface as organization-held legal knowledge");
    assert!(
        legal_knowledge.summary().contains("Vice") || legal_knowledge.summary().contains("vice")
    );

    // The next cycle pays compounded street heat: the pressure case and the new inquiry both
    // tax the district while they stay open, and the changed cost is manager-report-worthy.
    let pressure = settle_cycle_inner(&registry, &mut fixture, enterprise);
    assert_eq!(pressure, AttentionClass::Notable);
    let settled_cycle = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .max_by_key(|cycle| cycle.occurred_at())
        .expect("compounded cycle should persist");
    assert_eq!(
        settled_cycle.investigation_heat(),
        Money::from_cents(10_000),
        "two active district cases must compound the authored surcharge"
    );

    validate_state(&fixture.state).expect("vice-attention state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn non_police_enterprise_case_does_not_suppress_first_police_vice_inquiry() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police =
        insert_district_police(&registry, &mut fixture, "Police Vice Intake", neighborhood);
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Police district pressure",
        neighborhood,
    );

    // A public legal authority may own its own investigation into the same enterprise, but it is
    // not a police vice inquiry and must not consume the police visibility event.
    let legal_authority = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Public Regulatory Authority".to_owned(),
            kind: OrganizationKind::LegalAuthority,
        },
    )
    .expect("legal-authority fixture should validate");
    validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: legal_authority,
            title: "Regulatory enterprise inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Enterprise(enterprise)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Enterprise(enterprise),
                origin: Some(EntityRef::Enterprise(enterprise)),
                kind: EvidenceKind::FinancialRecord,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Enterprise(enterprise)),
            notified_organizations: BTreeSet::from([fixture.organization]),
            witness: None,
        },
    )
    .expect("legal-authority incident should validate")
    .commit(&mut fixture.state)
    .expect("legal-authority incident should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("police-pressure cycle should resolve");
    let cycle = validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("police-pressure cycle should validate")
        .commit(&mut fixture.state)
        .expect("police-pressure cycle should commit");
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(cycle)
            .expect("cycle should persist")
            .drew_vice_attention(),
        "a non-police enterprise investigation must not suppress the first real police vice inquiry"
    );
    assert!(find_vice_investigation(&fixture, police, enterprise).is_some());
    validate_state(&fixture.state).expect("mixed-authority enterprise casework should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("mixed-authority enterprise casework should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn validated_vice_cycle_stales_atomically_when_intake_priority_changes() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let original_police = insert_district_police(
        &registry,
        &mut fixture,
        "Original Vice Bureau",
        neighborhood,
    );
    open_district_pressure_case(
        &registry,
        &mut fixture,
        original_police,
        "Routing pressure case",
        neighborhood,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("vice-producing cycle should resolve");
    let validated = validate_enterprise_cycle_plan(&fixture.state, plan)
        .expect("vice-producing cycle should initially validate");
    let cycles_before = fixture.state.enterprises().cycles_for(enterprise).count();
    let investigations_before = fixture.state.legal().active_investigations().count();
    let information_before = fixture
        .state
        .intelligence()
        .information_for_holder(KnowledgeHolder::Organization(fixture.organization))
        .count();
    let cash_before = fixture
        .state
        .finance()
        .get_account(fixture.cash)
        .expect("enterprise cash should exist")
        .balance();
    let settlement_before = fixture
        .state
        .finance()
        .get_account(fixture.settlement)
        .expect("enterprise settlement should exist")
        .balance();

    // A different bureau takes intake after the whole cycle has validated but before commit.
    // The held token must not open a fresh case under its stale authority snapshot.
    let replacement_police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Replacement Vice Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("replacement bureau should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: replacement_police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(90),
        },
    )
    .expect("replacement jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("replacement jurisdiction should commit");
    assert_eq!(
        crate::legal::jurisdiction_system::resolve_case_intake_authority(
            &fixture.state,
            neighborhood,
        ),
        Some(replacement_police)
    );

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("changed intake priority must stale the validated vice cycle");
    assert_eq!(
        error,
        EnterpriseError::StaleViceIntakeRouting {
            enterprise,
            neighborhood,
            expected: Some(original_police),
            found: Some(replacement_police),
        }
    );
    assert_eq!(
        fixture.state.enterprises().cycles_for(enterprise).count(),
        cycles_before
    );
    assert_eq!(
        fixture.state.legal().active_investigations().count(),
        investigations_before
    );
    assert_eq!(
        fixture
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(fixture.organization))
            .count(),
        information_before,
        "stale routing must reject before cycle or vice information is published"
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("enterprise cash should persist")
            .balance(),
        cash_before
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .expect("enterprise settlement should persist")
            .balance(),
        settlement_before
    );
    assert!(find_vice_investigation(&fixture, original_police, enterprise).is_none());
    assert!(find_vice_investigation(&fixture, replacement_police, enterprise).is_none());
    validate_state(&fixture.state).expect("stale vice routing rejection must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn planned_vice_cycle_stales_when_same_intake_authority_changes_version() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Versioned Vice Bureau",
        neighborhood,
    );
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Version pressure case",
        neighborhood,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("vice-producing plan should resolve");

    // The same organization still wins intake, but changing its priority bumps the jurisdiction
    // version. A plan computed from the old routing context must re-decide before validation.
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: rating(85),
        },
    )
    .expect("same-authority jurisdiction revision should validate")
    .commit(&mut fixture.state)
    .expect("same-authority jurisdiction revision should commit");
    assert_eq!(
        crate::legal::jurisdiction_system::resolve_case_intake_authority(
            &fixture.state,
            neighborhood,
        ),
        Some(police)
    );

    let error = match validate_enterprise_cycle_plan(&fixture.state, plan) {
        Ok(_) => panic!("changed jurisdiction version must stale the planned vice cycle"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        EnterpriseError::StaleViceIntakeJurisdictionVersion {
            enterprise,
            neighborhood,
            organization: police,
            expected_version: 1,
            found_version: Some(2),
        }
    );
    assert_eq!(
        fixture.state.enterprises().cycles_for(enterprise).count(),
        0
    );
    assert!(find_vice_investigation(&fixture, police, enterprise).is_none());
    validate_state(&fixture.state).expect("stale planned vice cycle must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn hot_district_without_current_intake_does_not_emit_phantom_vice_attention() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Departing Vice Bureau",
        neighborhood,
    );
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Persistent old pressure",
        neighborhood,
    );

    // Establish the current positive heat level without drawing a dedicated inquiry.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("initial hot cycle should resolve");
    assert!(first.investigation_heat() > Money::ZERO);
    validate_enterprise_cycle_plan(&fixture.state, first)
        .expect("initial hot cycle should validate")
        .commit(&mut fixture.state)
        .expect("initial hot cycle should commit");

    // The bureau gives up this district but its already-open pressure case remains active. Heat
    // therefore persists, while no institution currently exists to own a new vice inquiry.
    let elsewhere = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Elsewhere Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(50),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(50),
                },
            },
        },
    )
    .expect("replacement neighborhood should validate");
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([elsewhere]),
            case_intake_priority: rating(80),
        },
    )
    .expect("jurisdiction withdrawal should validate")
    .commit(&mut fixture.state)
    .expect("jurisdiction withdrawal should commit");
    assert_eq!(
        crate::legal::jurisdiction_system::resolve_case_intake_authority(
            &fixture.state,
            neighborhood,
        ),
        None
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let unroutable = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("unroutable hot cycle should still resolve");
    assert!(unroutable.investigation_heat() > Money::ZERO);
    assert_eq!(
        unroutable.attention(),
        AttentionClass::Routine,
        "an unchanged heat surcharge plus an unroutable vice roll is not fresh manager news"
    );
    let cycle = validate_enterprise_cycle_plan(&fixture.state, unroutable)
        .expect("unroutable hot cycle should validate")
        .commit(&mut fixture.state)
        .expect("unroutable hot cycle should commit");
    let cycle = fixture
        .state
        .enterprises()
        .get_cycle(cycle)
        .expect("unroutable cycle should persist");
    assert!(!cycle.drew_vice_attention());
    assert_eq!(cycle.attention(), AttentionClass::Routine);
    assert!(find_vice_investigation(&fixture, police, enterprise).is_none());
    validate_state(&fixture.state).expect("unroutable vice state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("unroutable vice cycle must remain registry-valid");
    validate_invariants(&fixture.state);

    // Absence is routing state too. Hold a fully validated plan whose vice roll hit while no
    // authority covered the district, then restore police intake before commit. The token must
    // stale rather than silently settle the now-routable roll as though nothing changed.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let unrouted_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("second unroutable hot plan should resolve");
    let validated = validate_enterprise_cycle_plan(&fixture.state, unrouted_plan)
        .expect("absent intake routing should validate against its own snapshot");
    let cycles_before = fixture.state.enterprises().cycles_for(enterprise).count();
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood, elsewhere]),
            case_intake_priority: rating(80),
        },
    )
    .expect("restored district jurisdiction should validate")
    .commit(&mut fixture.state)
    .expect("restored district jurisdiction should commit");
    let error = validated
        .commit(&mut fixture.state)
        .expect_err("new intake authority must stale a validated unroutable vice cycle");
    assert_eq!(
        error,
        EnterpriseError::StaleViceIntakeRouting {
            enterprise,
            neighborhood,
            expected: None,
            found: Some(police),
        }
    );
    assert_eq!(
        fixture.state.enterprises().cycles_for(enterprise).count(),
        cycles_before,
        "routing staleness must reject before a cycle is persisted"
    );
    assert!(find_vice_investigation(&fixture, police, enterprise).is_none());
    validate_state(&fixture.state)
        .expect("absent-to-present routing rejection must preserve state");
    validate_invariants(&fixture.state);
}

#[test]
fn shelved_vice_inquiry_releases_the_racket_from_compounded_heat() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let police = insert_district_police(&registry, &mut fixture, "Cold Ward Police", neighborhood);
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Market ward heat",
        neighborhood,
    );

    // Draw the vice inquiry with the strongest roll alignment.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let hot_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("hot cycle plan should resolve");
    validate_enterprise_cycle_plan(&fixture.state, hot_plan)
        .expect("hot cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("hot cycle settlement should commit");
    assert!(find_vice_investigation(&fixture, police, enterprise).is_some());

    // With no institutional activity, cold-case decay shelves every originated case in the
    // district. The clock jumps past the authored inactivity window without settling further
    // cycles, so the decay sees exactly the two cases opened above.
    fixture.state.advance_clock(SimDuration::from_minutes(
        registry.legal().cold_case_window().as_minutes() + 1,
    ));
    crate::legal::investigation_system::apply_cold_case_decay(
        &mut fixture.state,
        registry.legal().cold_case_window(),
    )
    .expect("cold-case decay should resolve");
    let shelved = find_vice_investigation(&fixture, police, enterprise)
        .expect("the vice case record must persist after shelving");
    let shelved_id = shelved.id();
    assert_eq!(
        shelved.status(),
        crate::legal::InvestigationStatus::Suspended,
        "an inactive vice inquiry must shelf like any other originated case"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let quiet_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("quiet cycle plan should resolve");
    assert_eq!(
        quiet_plan.investigation_heat(),
        Money::ZERO,
        "shelved casework must stop taxing the racket"
    );
    assert_eq!(
        quiet_plan.attention(),
        AttentionClass::Notable,
        "leadership must be told when a previously visible street surcharge clears"
    );
    let quiet_cycle = validate_enterprise_cycle_plan(&fixture.state, quiet_plan)
        .expect("quiet recovery cycle should validate")
        .commit(&mut fixture.state)
        .expect("quiet recovery cycle should commit");
    let recovery_information = fixture
        .state
        .enterprises()
        .get_cycle(quiet_cycle)
        .and_then(|cycle| cycle.information())
        .and_then(|information| fixture.state.intelligence().get_information(information))
        .expect("heat recovery must persist a manager-facing information record");
    assert!(
        recovery_information
            .summary()
            .contains("prior street surcharge cleared as police work eased"),
        "recovery report must explain why operating cost fell: {}",
        recovery_information.summary()
    );
    validate_state(&fixture.state).expect("post-decay state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("heat recovery report must remain registry-valid");

    // Fresh district pressure after the shelf may draw vice attention again. Canonical intake
    // must reopen the existing enterprise shelf, not manufacture a second case, while retaining
    // the original incident evidence that proves the first vice-drawing cycle.
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Renewed market ward heat",
        neighborhood,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let resumed_cycle = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, 0),
        )
        .expect("renewed pressure cycle should resolve"),
    )
    .expect("renewed pressure cycle should validate")
    .commit(&mut fixture.state)
    .expect("renewed pressure cycle should commit");
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(resumed_cycle)
            .expect("resumed-pressure cycle should persist")
            .drew_vice_attention()
    );
    let resumed = find_vice_investigation(&fixture, police, enterprise)
        .expect("renewed vice attention should reactivate the shelved inquiry");
    assert_eq!(
        resumed.id(),
        shelved_id,
        "intake must resume the same case file"
    );
    assert_eq!(resumed.status(), crate::legal::InvestigationStatus::Active);
    assert_eq!(
        resumed.evidence().len(),
        2,
        "the resumed vice file must retain its original incident evidence and add the new hit"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .investigations_for_subject(EntityRef::Enterprise(enterprise))
            .filter(|investigation| {
                investigation.owner() == police
                    && investigation.origin() == Some(EntityRef::Enterprise(enterprise))
            })
            .count(),
        1,
        "shelf resumption must not create a parallel police inquiry"
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("both vice-drawing cycles should remain structurally saveable"),
    )
    .expect("resumed vice history should restore with both evidence timestamps intact");
    validate_state_against_registry(&registry, &restored)
        .expect("resumed vice history must remain registry-valid after restore");
    validate_invariants(&restored);
}
