//! Focused tests for `enterprise_execution` lifecycle, settlement, and reporting.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{
    StateValidationError, validate_invariants, validate_state, validate_state_against_registry,
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
use crate::enterprises::enterprise_reporting::resolve_organization_enterprise_financial_summary;
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
};
use crate::intelligence::InformationTopic;
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{
    apply_cold_case_decay, validate_add_evidence, validate_incident_intake,
    validate_open_investigation,
};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft, JurisdictionDraft,
};
use crate::operations::operation_system::{
    OperationTransition, apply_transition, validate_authorize_operation,
};
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
    OrganizationDraft, OrganizationKind, Rating,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

mod autonomous_expansion;

struct EnterpriseFixture {
    state: AppState,
    authority: MandateAuthority,
    organization: OrganizationId,
    location: EnterpriseLocation,
    cash: FinancialAccountId,
    settlement: FinancialAccountId,
}

#[derive(Clone, Serialize)]
struct EnterpriseIdentityWire {
    id: EnterpriseId,
    kind: EnterpriseKind,
}

#[derive(Clone, Serialize)]
struct EnterpriseAssignmentWire {
    organization: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
    supporting_businesses: BTreeSet<BusinessId>,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Serialize)]
struct EnterpriseRuntimeWire {
    status: EnterpriseStatus,
    established_at: SimTime,
    retired_at: Option<SimTime>,
    next_cycle_at: Option<SimTime>,
    last_cycle_at: Option<SimTime>,
    loss_streak_anchor: Option<SimTime>,
    version: u32,
}

#[derive(Clone, Serialize)]
struct EnterpriseRecordWire {
    identity: EnterpriseIdentityWire,
    assignment: EnterpriseAssignmentWire,
    runtime: EnterpriseRuntimeWire,
}

fn enterprise_wire(record: &crate::enterprises::EnterpriseRecord) -> EnterpriseRecordWire {
    EnterpriseRecordWire {
        identity: EnterpriseIdentityWire {
            id: record.id(),
            kind: record.kind(),
        },
        assignment: EnterpriseAssignmentWire {
            organization: record.organization(),
            authority: record.authority(),
            location: record.location(),
            supporting_businesses: record.supporting_businesses().clone(),
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
        },
        runtime: EnterpriseRuntimeWire {
            status: record.status(),
            established_at: record.established_at(),
            retired_at: record.retired_at(),
            next_cycle_at: record.next_cycle_at(),
            last_cycle_at: record.last_cycle_at(),
            loss_streak_anchor: record.loss_streak_anchor(),
            version: record.version(),
        },
    }
}

fn replace_serialized_enterprise(
    envelope: SaveEnvelope,
    original: &crate::enterprises::EnterpriseRecord,
    replacement: &EnterpriseRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("enterprise should serialize");
    let mirror = enterprise_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("enterprise mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement enterprise should serialize");
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
        "serialized enterprise must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout enterprise corruption must remain decodable")
}

#[test]
fn restore_rejects_duplicate_non_retired_enterprise_kind_at_location() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                fixture.authority.scope,
                ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("broad enterprise authority should validate")
    .commit(&mut fixture.state)
    .expect("broad enterprise authority should commit");

    let first = establish_protection(&registry, &mut fixture);
    let second_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Second Enterprise Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(55),
                    commercial_activity: rating(60),
                    illicit_demand: rating(45),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(35),
                },
            },
        },
    )
    .expect("second neighborhood should validate");
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second settlement account should validate");
    let second = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: MandateAuthority {
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
                ..fixture.authority
            },
            location: EnterpriseLocation::Neighborhood(second_neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: second_settlement,
        },
    )
    .expect("same kind at another location should validate")
    .commit(&mut fixture.state)
    .expect("same kind at another location should commit");
    validate_state(&fixture.state)
        .expect("canonical distinct-location enterprises should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("canonical distinct-location enterprises should match authored content");

    let second_record = fixture
        .state
        .enterprises()
        .get_enterprise(second)
        .expect("second enterprise should persist");
    let mut corrupted = enterprise_wire(second_record);
    corrupted.assignment.location = fixture.location;
    let corrupted_envelope = replace_serialized_enterprise(
        build_save(&registry, &fixture.state).expect("canonical enterprise state should save"),
        second_record,
        &corrupted,
    );

    let error = restore_save(&registry, corrupted_envelope)
        .expect_err("restore must reject a duplicate non-retired enterprise slot");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::DuplicateEnterpriseLocation {
            enterprise: second,
            existing: first,
        })
    );
}

#[derive(Clone, Serialize)]
struct BusinessRecordWire {
    id: BusinessId,
    name: String,
    kind: BusinessKind,
    functions: BTreeSet<BusinessFunction>,
    neighborhood: NeighborhoodId,
    owner: BusinessOwner,
    version: u32,
}

#[test]
fn restore_rejects_active_enterprise_schedule_drift_from_authored_cadence() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
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
    .expect("routine enterprise cycle should commit");

    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("active enterprise should persist after settlement");
    let valid_next = record
        .next_cycle_at()
        .expect("ordinary post-settlement enterprise should remain scheduled");
    let mut corrupted = enterprise_wire(record);
    corrupted.runtime.next_cycle_at = Some(valid_next + SimDuration::ONE_MINUTE);
    let error = restore_save(
        &registry,
        replace_serialized_enterprise(
            build_save(&registry, &fixture.state)
                .expect("valid scheduled enterprise should save before corruption"),
            record,
            &corrupted,
        ),
    )
    .expect_err("restore must reject an enterprise schedule canonical cadence cannot produce");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidEnterpriseSchedule {
                enterprise: invalid
            }
        ) if invalid == enterprise
    ));
}

fn business_wire(record: &crate::world::BusinessRecord) -> BusinessRecordWire {
    BusinessRecordWire {
        id: record.id(),
        name: record.name().to_owned(),
        kind: record.kind(),
        functions: record.functions().clone(),
        neighborhood: record.neighborhood(),
        owner: record.owner(),
        version: record.version(),
    }
}

#[test]
fn due_enterprise_cycle_near_clock_horizon_settles_then_exhausts_future_recurrence() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let cycle_duration = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .cycle();
    let settled_at = SimTime::from_minutes(u64::MAX - u64::from(cycle_duration.as_minutes()) + 1);
    fixture.state.set_now_for_test(settled_at);

    let cycle = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect("already-due enterprise work should settle even when only its next recurrence overflows"),
    )
    .expect("horizon enterprise cycle should validate")
    .commit(&mut fixture.state)
    .expect("horizon enterprise cycle should commit");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_cycle(cycle)
            .expect("horizon cycle should persist")
            .occurred_at(),
        settled_at
    );
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should remain live");
    assert_eq!(record.status(), EnterpriseStatus::Active);
    assert_eq!(record.last_cycle_at(), Some(settled_at));
    assert_eq!(record.next_cycle_at(), None);
    assert!(
        find_due_enterprises(&fixture.state).is_empty(),
        "an exhausted recurrence must leave no same-minute enterprise schedule behind"
    );
    assert_eq!(
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect_err("no second enterprise settlement is representable after recurrence exhaustion"),
        EnterpriseError::SimulationTimeOverflow
    );
    validate_state(&fixture.state)
        .expect("recurrence-exhausted enterprise should remain structurally valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("authored-overflow enterprise recurrence should be registry-valid");
    validate_invariants(&fixture.state);

    let mut suspended = fixture.state.clone();
    validate_suspend_enterprise(&suspended, enterprise)
        .expect("recurrence exhaustion must not prevent an explicit enterprise suspension")
        .commit(&mut suspended)
        .expect("unscheduled active enterprise should suspend without a stale due-index entry");
    let suspended_record = suspended
        .enterprises()
        .get_enterprise(enterprise)
        .expect("suspended horizon enterprise should persist");
    assert_eq!(suspended_record.status(), EnterpriseStatus::Suspended);
    assert_eq!(suspended_record.next_cycle_at(), None);
    validate_state(&suspended)
        .expect("suspended horizon enterprise should remain structurally valid");
    validate_state_against_registry(&registry, &suspended)
        .expect("suspended recurrence-exhausted enterprise should remain registry-valid");
    validate_invariants(&suspended);

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("recurrence-exhausted enterprise should remain saveable"),
    )
    .expect("enterprise recurrence exhaustion must survive restore");
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .expect("restored enterprise should persist")
            .next_cycle_at(),
        None
    );
}

fn replace_serialized_business(
    envelope: SaveEnvelope,
    original: &crate::world::BusinessRecord,
    replacement: &BusinessRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("business record should serialize");
    let mirror = business_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("business mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement business should serialize");
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
        "serialized business must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout business corruption must remain decodable")
}

#[derive(Clone, Serialize)]
struct BusinessOwnershipChangeRecordWire {
    id: crate::core::id::BusinessOwnershipChangeId,
    business: BusinessId,
    previous_owner: Option<BusinessOwner>,
    new_owner: BusinessOwner,
    changed_at: SimTime,
    resulting_business_version: u32,
}

fn ownership_change_wire(
    record: &crate::world::BusinessOwnershipChangeRecord,
) -> BusinessOwnershipChangeRecordWire {
    BusinessOwnershipChangeRecordWire {
        id: record.id(),
        business: record.business(),
        previous_owner: record.previous_owner(),
        new_owner: record.new_owner(),
        changed_at: record.changed_at(),
        resulting_business_version: record.resulting_business_version(),
    }
}

fn replace_serialized_ownership_change(
    envelope: SaveEnvelope,
    original: &crate::world::BusinessOwnershipChangeRecord,
    replacement: &BusinessOwnershipChangeRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("ownership change should serialize");
    let mirror = ownership_change_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("ownership-change mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement ownership change should serialize");
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
        "serialized ownership change must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout ownership corruption must remain decodable")
}

/// Opens a district-pressure case from an operation that actually occurred. The provenance
/// operation is started through the simulation tick and then stood down through the canonical
/// authority transition so it releases its participant instead of leaving a synthetic,
/// permanently-authorized booking behind.
#[derive(Clone, Copy)]
struct PressureCaseOrigin {
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
}

fn open_originated_pressure_case(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    police: OrganizationId,
    case_origin: PressureCaseOrigin,
    title: &str,
    target: EntityRef,
) {
    let PressureCaseOrigin {
        organization: origin_organization,
        manager: origin_manager,
    } = case_origin;
    let origin = validate_authorize_operation(
        registry,
        &fixture.state,
        OperationDraft {
            title: format!("{title} origin patrol"),
            kind: OperationKind::Surveillance,
            responsible_organization: origin_organization,
            leader: origin_manager,
            objective: OperationObjective::GatherInformation { target },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, origin_manager)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("origin operation should validate")
    .commit(&mut fixture.state)
    .expect("origin operation should commit");
    let start = run_tick(registry, &mut fixture.state);
    assert!(
        start.started_operations.contains(&origin),
        "origin operation must actually begin before it can ground a pressure case"
    );
    apply_transition(
        registry,
        &mut fixture.state,
        origin,
        OperationTransition::Abort,
    )
    .expect("origin attempt should stand down cleanly after beginning");
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
            witness: None,
        },
    )
    .expect("pressure case intake should validate")
    .commit(&mut fixture.state)
    .expect("pressure case intake should commit");
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

#[test]
fn held_establishment_rejects_clock_overflow_before_planned_account_or_enterprise_id_is_consumed() {
    use crate::core::id::IdKind;
    use crate::finance::finance_system::validate_open_accounts;

    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let openings = validate_open_accounts(
        &fixture.state,
        vec![FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        }],
    )
    .expect("fresh settlement opening should validate read-only");
    let fresh_settlement = openings
        .account_id(0)
        .expect("one-account opening should expose its predicted id");
    let validated = validate_establish_enterprise_with_openings(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: fixture.authority,
            location: fixture.location,
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fresh_settlement,
        },
        openings,
    )
    .expect("enterprise should validate before the clock moves");
    let cycle = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .cycle();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(cycle.as_minutes()) + 1,
    ));
    let account_next = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let enterprise_next = fixture.state.ids.next_raw(IdKind::Enterprise);

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("held enterprise token must reject before unrepresentable cycle scheduling");
    assert_eq!(error, EnterpriseError::SimulationTimeOverflow);
    assert!(
        fixture
            .state
            .finance()
            .get_account(fresh_settlement)
            .is_none(),
        "planned settlement account must remain unopened"
    );
    assert_eq!(fixture.state.enterprises().enterprises().count(), 0);
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        account_next
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::Enterprise),
        enterprise_next
    );
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

    // The load boundary must reject a cycle whose vice flag disagrees with canonical incident
    // evidence. The flag is fixed-width, so restore reaches structural validation rather than
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
    .expect_err("a cycle cannot deny a vice event that has canonical incident evidence");
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
        plan.economics.net_cash,
        plan.economics
            .gross_revenue
            .checked_sub(plan.economics.operating_cost)
            .expect("net should be gross - cost")
    );
    assert!(plan.economics.gross_revenue.cents() > 0);
    assert!(plan.economics.operating_cost.cents() > 0);

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
fn autonomous_expansion_honors_organization_wide_enterprise_function_authority() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    fund_enterprise_fixture_cash(&mut fixture, 100_000);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let enterprise_scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([enterprise_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("enterprise-function mandate should validate")
    .commit(&mut fixture.state)
    .expect("enterprise-function mandate should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let established = apply_due_autonomous_enterprises(&registry, &mut fixture.state)
        .expect("broad enterprise authority should support autonomous expansion");
    assert_eq!(established.len(), 1);
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(established[0])
        .expect("function-authorized enterprise should persist");
    assert_eq!(record.authority().scope, enterprise_scope);
    assert_eq!(
        record.location(),
        EnterpriseLocation::Neighborhood(neighborhood)
    );
    validate_state(&fixture.state).expect("function-authorized expansion state should validate");
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
    let open_heat_case = |fixture: &mut EnterpriseFixture, title: &str, target| {
        open_originated_pressure_case(
            &registry,
            fixture,
            police,
            PressureCaseOrigin {
                organization: fixture.organization,
                manager: fixture.authority.manager,
            },
            title,
            target,
        );
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
            plan.economics.operating_cost,
            plan.economics.investigation_heat,
            plan.economics.attention,
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
    let open_case = |fixture: &mut EnterpriseFixture, title: &str| {
        open_originated_pressure_case(
            &registry,
            fixture,
            police,
            PressureCaseOrigin {
                organization: fixture.organization,
                manager: fixture.authority.manager,
            },
            title,
            EntityRef::Neighborhood(local_neighborhood),
        );
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

#[test]
fn cycle_plan_rejects_when_district_case_pressure_changes_before_settlement() {
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
    .expect("quiet due cycle should resolve");

    let police = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Pressure Snapshot Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: police,
            title: "Fresh enterprise inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Enterprise(enterprise)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Enterprise(enterprise),
                origin: Some(EntityRef::Enterprise(enterprise)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Enterprise(enterprise)),
            witness: None,
        },
    )
    .expect("fresh pressure case should validate")
    .commit(&mut fixture.state)
    .expect("fresh pressure case should commit without advancing time");

    let error = match validate_enterprise_cycle_plan(&fixture.state, plan) {
        Err(error) => error,
        Ok(_) => panic!("held cycle must stale when legal pressure changes"),
    };
    assert!(matches!(
        error,
        EnterpriseError::StaleLegalPressureContext {
            enterprise: stale_enterprise,
            expected_active_district_cases: 0,
            found_active_district_cases: 1,
            expected_active_inquiry: false,
            found_active_inquiry: true,
        } if stale_enterprise == enterprise
    ));
    assert!(
        fixture
            .state
            .enterprises()
            .cycles_for(enterprise)
            .next()
            .is_none(),
        "stale plan rejection must not settle a cycle"
    );
    validate_state(&fixture.state).expect("rejected stale plan leaves valid state");
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
    let attention = plan.economics.attention;
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
    let corroborating = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(manager),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("corroborating custody evidence should validate")
    .commit(&mut fixture.state)
    .expect("corroborating custody evidence should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let stale_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("due cycle should plan while the manager is free");
    let arrest = validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: manager,
            investigation,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("manager arrest should not require revoking formal enterprise authority")
    .commit(&mut fixture.state)
    .expect("manager arrest should commit");

    let stale_error = match validate_enterprise_cycle_plan(&fixture.state, stale_plan) {
        Err(error) => error,
        Ok(_) => panic!("arrest must stale a cycle planned while the manager was free"),
    };
    assert_eq!(
        stale_error,
        EnterpriseError::Delegation(
            crate::delegation::delegation_system::DelegationError::DetainedManager {
                manager,
                arrest,
            },
        )
    );
    assert_eq!(
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, u16::MAX),
        )
        .expect_err("a detained manager cannot settle through the direct enterprise API"),
        EnterpriseError::Delegation(
            crate::delegation::delegation_system::DelegationError::DetainedManager {
                manager,
                arrest,
            },
        )
    );

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
        plan.economics.net_cash,
        plan.economics
            .gross_revenue
            .checked_sub(plan.economics.operating_cost)
            .expect("net should be gross - cost")
    );
    assert!(plan.economics.gross_revenue.cents() > plan.economics.operating_cost.cents());
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
fn due_enterprises_preserve_schedule_chronology_before_id_order() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let later_due_lower_id = establish_protection(&registry, &mut fixture);

    let organization = fixture.organization;
    let venue = insert_support_business(
        &registry,
        &mut fixture,
        "Chronology Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second settlement account should validate");
    let earlier_due_higher_id = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: second_settlement,
        },
    )
    .expect("second enterprise should validate")
    .commit(&mut fixture.state)
    .expect("second enterprise should commit");
    assert!(later_due_lower_id < earlier_due_higher_id);

    // Both rackets were created at the same instant. Rescheduling only the lower-ID racket
    // makes its next cycle later. Once both are overdue, scheduler order must remain
    // (due time, ID), not collapse back to raw creation order.
    validate_suspend_enterprise(&fixture.state, later_due_lower_id)
        .expect("lower-ID enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("lower-ID suspension should commit");
    fixture.state.advance_clock(SimDuration::from_minutes(60));
    validate_resume_enterprise(&registry, &fixture.state, later_due_lower_id)
        .expect("lower-ID enterprise should resume")
        .commit(&mut fixture.state)
        .expect("lower-ID resume should commit");

    let first_due_at = fixture
        .state
        .enterprises()
        .get_enterprise(earlier_due_higher_id)
        .and_then(|record| record.next_cycle_at())
        .expect("higher-ID enterprise should remain scheduled");
    let second_due_at = fixture
        .state
        .enterprises()
        .get_enterprise(later_due_lower_id)
        .and_then(|record| record.next_cycle_at())
        .expect("resumed lower-ID enterprise should be scheduled");
    assert!(first_due_at < second_due_at);
    let catch_up_minutes =
        u32::try_from(second_due_at.as_minutes() - fixture.state.now().as_minutes())
            .expect("fixture catch-up duration must fit SimDuration");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(catch_up_minutes));

    assert_eq!(
        find_due_enterprises(&fixture.state),
        vec![earlier_due_higher_id, later_due_lower_id],
        "older overdue work must consume the earlier deterministic cycle slot even when its ID is higher"
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
fn organization_financial_reporting_rederives_cycle_totals_without_cached_state() {
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
    let organization_summary = resolve_organization_enterprise_financial_summary(
        &fixture.state,
        fixture.organization,
        period_start,
        period_end,
    )
    .expect("organization financial summary should resolve");

    assert_eq!(organization_summary.totals.enterprise_count, 1);
    assert_eq!(organization_summary.totals.cycle_count, 2);
    assert_eq!(organization_summary.totals.notable_cycle_count, 1);
    assert_eq!(
        organization_summary
            .by_kind
            .get(&EnterpriseKind::Protection)
            .expect("protection bucket should exist"),
        &organization_summary.totals
    );
    let cycle_net = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .try_fold(Money::ZERO, |total, cycle| {
            total.checked_add(cycle.net_cash())
        })
        .expect("reporting fixture total should not overflow");
    assert_eq!(organization_summary.totals.net_cash, cycle_net);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.cash)
            .expect("cash account should exist")
            .balance(),
        organization_summary.totals.net_cash
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
            plan.economics.net_cash.cents() < 0,
            "fixture must produce a losing settlement"
        );
        assert_eq!(plan.economics.attention, AttentionClass::Notable);
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
        assert!(plan.economics.net_cash.cents() < 0);
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
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("retired history should persist")
            .retired_at(),
        Some(fixture.state.now())
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
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .expect("retired enterprise should survive restore")
            .retired_at(),
        Some(fixture.state.now())
    );
    validate_state_against_registry(&registry, &restored)
        .expect("retired enterprise history should remain registry-valid");
    crate::core::invariants::validate_invariants(&restored);
}

#[test]
fn restore_rejects_active_enterprise_at_foreign_owned_host() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let rival = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Foreign Venue Owner".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("foreign owner fixture should validate");
    let organization = fixture.organization;
    let venue = insert_support_business(
        &registry,
        &mut fixture,
        "Invariant Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: fixture.settlement,
        },
    )
    .expect("owned hosted racket should validate")
    .commit(&mut fixture.state)
    .expect("owned hosted racket should commit");

    let original = fixture
        .state
        .world()
        .get_business(venue)
        .expect("host business should persist");
    let ownership = fixture
        .state
        .world()
        .get_business_ownership_change_for_version(venue, original.version())
        .expect("host's current ownership record should persist");
    let mut corrupted = business_wire(original);
    corrupted.owner = BusinessOwner::Organization(rival);
    let mut corrupted_ownership = ownership_change_wire(ownership);
    corrupted_ownership.new_owner = BusinessOwner::Organization(rival);
    let envelope = build_save(&registry, &fixture.state)
        .expect("valid hosted enterprise should save before corruption");
    let envelope = replace_serialized_business(envelope, original, &corrupted);
    let envelope = replace_serialized_ownership_change(envelope, ownership, &corrupted_ownership);
    let error = restore_save(&registry, envelope)
        .expect_err("an active racket cannot survive restore at a foreign-owned host");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidEnterpriseAuthority {
                enterprise: invalid,
            }
        ) if invalid == enterprise
    ));
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
    open_originated_pressure_case(
        registry,
        fixture,
        police,
        PressureCaseOrigin {
            organization: fixture.organization,
            manager: fixture.authority.manager,
        },
        title,
        EntityRef::Neighborhood(neighborhood),
    );
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
fn same_minute_peer_vice_inquiry_does_not_retroactively_raise_cycle_heat() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let protection = establish_protection(&registry, &mut fixture);
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(_) => panic!("fixture should use a neighborhood location"),
    };
    let organization = fixture.organization;
    let venue = insert_support_business(
        &registry,
        &mut fixture,
        "Peer Settlement Card Room",
        BusinessKind::Hospitality,
        BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
        ]),
        BusinessOwner::Organization(organization),
    );
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second settlement account should validate");
    let gambling = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: fixture.authority,
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: fixture.cash,
            settlement_account: second_settlement,
        },
    )
    .expect("peer gambling enterprise should validate")
    .commit(&mut fixture.state)
    .expect("peer gambling enterprise should commit");
    let police = insert_district_police(
        &registry,
        &mut fixture,
        "Peer Settlement Vice Bureau",
        neighborhood,
    );
    open_district_pressure_case(
        &registry,
        &mut fixture,
        police,
        "Preexisting peer pressure",
        neighborhood,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let first_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        protection,
        EnterpriseCycleRandomness::new(0, 0),
    )
    .expect("first hot peer cycle should resolve");
    assert_eq!(
        first_plan.economics.investigation_heat,
        Money::from_cents(5_000),
        "the one preexisting district case should price the first peer cycle"
    );
    let first_cycle = validate_enterprise_cycle_plan(&fixture.state, first_plan)
        .expect("first peer cycle should validate")
        .commit(&mut fixture.state)
        .expect("first peer cycle should commit");
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(first_cycle)
            .expect("first peer cycle should persist")
            .drew_vice_attention(),
        "the first peer should create the same-minute vice inquiry used by this regression"
    );

    let second_plan = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        gambling,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("second same-minute peer cycle should resolve");
    assert_eq!(
        second_plan.economics.investigation_heat,
        Money::from_cents(5_000),
        "a peer inquiry created by an earlier settlement at the same instant cannot retroactively tax this cycle"
    );
    validate_enterprise_cycle_plan(&fixture.state, second_plan)
        .expect(
            "same-minute peer cycle must remain valid against the phase-consistent pressure view",
        )
        .commit(&mut fixture.state)
        .expect("same-minute peer cycle should commit");

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let source_next_day = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        protection,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("next-day source cycle should resolve");
    assert_eq!(
        source_next_day.economics.investigation_heat,
        Money::from_cents(10_000),
        "the preexisting pressure case and yesterday's vice inquiry should both apply to the source racket"
    );
    let source_cycle = validate_enterprise_cycle_plan(&fixture.state, source_next_day)
        .expect("next-day source cycle should validate")
        .commit(&mut fixture.state)
        .expect("next-day source cycle should commit");
    assert!(
        !fixture
            .state
            .enterprises()
            .get_cycle(source_cycle)
            .expect("next-day source cycle should persist")
            .drew_vice_attention(),
        "an already-active inquiry must suppress a duplicate vice draw"
    );
    let next_day = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        gambling,
        EnterpriseCycleRandomness::new(0, u16::MAX),
    )
    .expect("next-day peer cycle should resolve");
    assert_eq!(
        next_day.economics.investigation_heat,
        Money::from_cents(10_000),
        "on the next cycle the preexisting case and yesterday's vice inquiry must both apply"
    );
    validate_state(&fixture.state).expect("peer settlement pressure state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("peer settlement economics should remain registry-valid");
    validate_invariants(&fixture.state);
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
    assert_eq!(clean_plan.economics.attention, AttentionClass::Routine);
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
    assert_eq!(hot_plan.economics.attention, AttentionClass::Notable);
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
    // Intake itself stays institutional. The manager can report the observable vice attention,
    // but formal case activity must be learned through surveillance/contact/legal channels.
    assert_eq!(
        fixture
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(fixture.organization),
                crate::intelligence::InformationTopic::LegalActivity,
            )
            .filter(|information| information.subject() == EntityRef::Enterprise(enterprise))
            .count(),
        0
    );
    let vice_cycle = fixture
        .state
        .enterprises()
        .cycles_for(enterprise)
        .max_by_key(|cycle| cycle.occurred_at())
        .expect("vice cycle should persist");
    let manager_information = fixture
        .state
        .intelligence()
        .get_information(
            vice_cycle
                .information()
                .expect("notable vice cycle must have a report"),
        )
        .expect("vice cycle manager information should persist");
    assert!(
        manager_information
            .summary()
            .contains("Vice officers were noticed watching")
    );
    assert!(!manager_information.summary().contains("case stays open"));

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
    assert!(first.economics.investigation_heat > Money::ZERO);
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
    assert!(unroutable.economics.investigation_heat > Money::ZERO);
    assert_eq!(
        unroutable.economics.attention,
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
        quiet_plan.economics.investigation_heat,
        Money::ZERO,
        "shelved casework must stop taxing the racket"
    );
    assert_eq!(
        quiet_plan.economics.attention,
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
            .investigations()
            .filter(|investigation| {
                investigation
                    .subjects()
                    .contains(&EntityRef::Enterprise(enterprise))
            })
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
