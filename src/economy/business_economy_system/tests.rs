//! Focused tests for business establishment, ownership transfer, cycle settlement, and reporting inputs.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_reporting::resolve_organization_business_financial_summary;
use crate::finance::finance_system::{
    LaunderingDraft, insert_account, validate_launder_funds, validate_record_transaction,
};
use crate::finance::{
    FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money,
};
use crate::reports::ReportKind;
use crate::reports::organization_financial_report::validate_organization_financial_report;
use crate::world::world_system::{
    insert_business, insert_neighborhood, insert_organization, validate_transfer_business_ownership,
};
use crate::world::{
    BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, NeighborhoodDraft,
    NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile,
    OrganizationDraft, OrganizationKind, Rating,
};
use serde::Serialize;
use std::collections::BTreeSet;

struct BusinessEconomyFixture {
    state: AppState,
    business: BusinessId,
    organization: crate::core::id::OrganizationId,
    operating: FinancialAccountId,
    settlement: FinancialAccountId,
}

#[test]
fn restore_rejects_active_business_schedule_drift_from_authored_cadence() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
            .expect("routine business cycle should decide"),
    )
    .expect("routine business cycle should validate")
    .commit(&mut fixture.state)
    .expect("routine business cycle should commit");

    let record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("active economy should persist after settlement");
    let valid_next = record
        .next_cycle_at()
        .expect("ordinary post-settlement economy should remain scheduled");
    let mut corrupted = business_economy_wire(record);
    corrupted.next_cycle_at = Some(valid_next + SimDuration::ONE_MINUTE);
    let error = restore_save(
        &registry,
        replace_serialized_economy(
            build_save(&registry, &fixture.state)
                .expect("valid scheduled economy should save before corruption"),
            record,
            &corrupted,
        ),
    )
    .expect_err(
        "restore must reject a plausible-looking schedule that canonical cadence cannot produce",
    );
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidBusinessEconomySchedule {
                business
            }
        ) if business == fixture.business
    ));
}

#[derive(Clone, Serialize)]
struct LedgerTransactionRecordWire {
    id: crate::core::id::LedgerTransactionId,
    occurred_at: SimTime,
    memo: String,
    postings: Vec<crate::finance::LedgerPosting>,
    budget_usage: Option<crate::finance::BudgetUsageRecord>,
}

#[test]
fn due_business_cycle_near_clock_horizon_settles_then_exhausts_future_recurrence() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    let cycle_duration = registry
        .get_business(BusinessKind::Retail)
        .economics()
        .cycle();
    let settled_at = SimTime::from_minutes(u64::MAX - u64::from(cycle_duration.as_minutes()) + 1);
    fixture.state.set_now_for_test(settled_at);

    let cycle = validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).expect(
            "already-due business work should settle even when only its next recurrence overflows",
        ),
    )
    .expect("horizon business cycle should validate")
    .commit(&mut fixture.state)
    .expect("horizon business cycle should commit");
    let cycle_record = fixture
        .state
        .economy()
        .get_cycle(cycle)
        .expect("horizon cycle should persist");
    assert_eq!(cycle_record.occurred_at(), settled_at);
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("business economy should remain live");
    assert_eq!(economy.status(), BusinessOperatingStatus::Active);
    assert_eq!(economy.last_cycle_at(), Some(settled_at));
    assert_eq!(economy.next_cycle_at(), None);
    assert!(
        find_due_businesses(&fixture.state).is_empty(),
        "an exhausted recurrence must leave no same-minute schedule behind"
    );
    assert_eq!(
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
            .expect_err("no second settlement is representable after recurrence exhaustion"),
        BusinessEconomyError::SimulationTimeOverflow
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("active economy with an authored-overflow recurrence should be registry-valid");
    validate_invariants(&fixture.state);

    let mut suspended = fixture.state.clone();
    validate_suspend_business_economy(&suspended, fixture.business)
        .expect("recurrence exhaustion must not prevent an explicit lifecycle suspension")
        .commit(&mut suspended)
        .expect(
            "unscheduled active economy should suspend without requiring a stale schedule index",
        );
    let suspended_record = suspended
        .economy()
        .get_business_economy(fixture.business)
        .expect("suspended horizon economy should persist");
    assert_eq!(
        suspended_record.status(),
        BusinessOperatingStatus::Suspended
    );
    assert_eq!(suspended_record.next_cycle_at(), None);
    validate_state_against_registry(&registry, &suspended)
        .expect("suspended recurrence-exhausted economy should remain registry-valid");
    validate_invariants(&suspended);

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("recurrence-exhausted economy should remain saveable"),
    )
    .expect("recurrence exhaustion must survive restore");
    assert_eq!(
        restored
            .economy()
            .get_business_economy(fixture.business)
            .expect("restored business economy should persist")
            .next_cycle_at(),
        None
    );
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

#[derive(Clone, Serialize)]
struct BusinessEconomyRecordWire {
    business: BusinessId,
    operating_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    status: crate::economy::BusinessOperatingStatus,
    established_at: SimTime,
    next_cycle_at: Option<SimTime>,
    last_cycle_at: Option<SimTime>,
    disrupted_through: Option<SimTime>,
    loss_streak_anchor: Option<SimTime>,
    laundered_this_cycle: Money,
    laundering_transactions_this_cycle: BTreeSet<crate::core::id::LedgerTransactionId>,
    version: u32,
}

#[test]
fn restore_rejects_nonincreasing_business_cycle_time_in_sequential_id_order() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    let settle = |fixture: &mut BusinessEconomyFixture| {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        validate_business_cycle_plan(
            &fixture.state,
            decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
                .expect("routine cycle should decide"),
        )
        .expect("routine cycle should validate")
        .commit(&mut fixture.state)
        .expect("routine cycle should commit")
    };
    let first_id = settle(&mut fixture);
    let second_id = settle(&mut fixture);
    let first = fixture
        .state
        .economy()
        .get_cycle(first_id)
        .expect("first cycle should persist");
    let second = fixture
        .state
        .economy()
        .get_cycle(second_id)
        .expect("second cycle should persist");
    assert!(first.id() < second.id());
    assert!(first.occurred_at() < second.occurred_at());
    assert_eq!(first.attention(), AttentionClass::Routine);
    assert!(first.information().is_none());
    let first_transaction_id = first
        .transaction()
        .expect("positive routine business cycle should carry a ledger settlement");
    let first_transaction = fixture
        .state
        .finance()
        .get_transaction(first_transaction_id)
        .expect("first settlement transaction should persist");

    // Move the older cycle and its ledger artifact forward to exactly the newer cycle's time.
    // Every local timestamp relationship still agrees. What becomes impossible is the owner's
    // documented settlement-order invariant: higher sequential cycle IDs must represent later
    // settlements for the same business.
    let mut corrupted_cycle = business_cycle_wire(first);
    corrupted_cycle.context.occurred_at = second.occurred_at();
    let mut corrupted_transaction = ledger_transaction_wire(first_transaction);
    corrupted_transaction.occurred_at = second.occurred_at();
    let envelope = replace_serialized_cycle(
        build_save(&registry, &fixture.state)
            .expect("valid two-cycle economy should save before chronology corruption"),
        first,
        &corrupted_cycle,
    );
    let envelope =
        replace_serialized_transaction(envelope, first_transaction, &corrupted_transaction);
    let error = restore_save(&registry, envelope)
        .expect_err("nonincreasing per-business cycle history must fail restore");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidBusinessCycle { cycle }
        ) if cycle == second_id
    ));
}

fn business_economy_wire(
    record: &crate::economy::BusinessEconomyRecord,
) -> BusinessEconomyRecordWire {
    BusinessEconomyRecordWire {
        business: record.business(),
        operating_account: record.operating_account(),
        settlement_account: record.settlement_account(),
        status: record.status(),
        established_at: record.established_at(),
        next_cycle_at: record.next_cycle_at(),
        last_cycle_at: record.last_cycle_at(),
        disrupted_through: record.disrupted_through(),
        loss_streak_anchor: record.loss_streak_anchor(),
        laundered_this_cycle: record.laundered_this_cycle(),
        laundering_transactions_this_cycle: record.laundering_transactions_this_cycle().clone(),
        version: record.version(),
    }
}

#[test]
fn restore_rejects_laundering_total_not_derived_from_linked_ledger_transactions() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let laundering = fund_and_launder(&registry, &mut fixture, Money::from_cents(500));
    let record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("laundered economy should persist");
    assert_eq!(record.laundered_this_cycle(), Money::from_cents(500));
    assert_eq!(
        record.laundering_transactions_this_cycle(),
        &BTreeSet::from([laundering])
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("valid laundering state should save"),
    )
    .expect("valid laundering provenance should restore");
    assert_eq!(
        restored
            .economy()
            .get_business_economy(fixture.business)
            .expect("restored laundering economy should persist")
            .laundered_this_cycle(),
        Money::from_cents(500)
    );

    let mut corrupted = business_economy_wire(record);
    corrupted.laundered_this_cycle = Money::ZERO;
    let error = restore_save(
        &registry,
        replace_serialized_economy(
            build_save(&registry, &fixture.state)
                .expect("valid laundering state should save before corruption"),
            record,
            &corrupted,
        ),
    )
    .expect_err("laundering capacity cannot be restored from an unauditable counter");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidBusinessEconomy { business }
        ) if business == fixture.business
    ));
}

#[test]
fn resume_starts_a_fresh_laundering_window() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fund_and_launder(&registry, &mut fixture, Money::from_cents(500));
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("laundered economy should persist")
            .laundered_this_cycle(),
        Money::from_cents(500)
    );
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .expect("laundered front should suspend")
        .commit(&mut fixture.state)
        .expect("laundered front should suspend atomically");
    validate_resume_business_economy(&registry, &fixture.state, fixture.business)
        .expect("suspended front should resume")
        .commit(&mut fixture.state)
        .expect("front resumption should commit atomically");
    let record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("resumed economy should persist");
    assert_eq!(record.laundered_this_cycle(), Money::ZERO);
    assert!(record.laundering_transactions_this_cycle().is_empty());
    validate_state_against_registry(&registry, &fixture.state)
        .expect("fresh laundering window after resume should restore safely");
}

#[test]
fn later_sabotage_does_not_retroactively_invalidate_prior_laundering() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let normal_gross =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("fixture gross should resolve");
    let normal_capacity = crate::finance::helpers::resolve_basis_point_share(
        normal_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("fixture laundering capacity should resolve");
    assert!(normal_capacity > Money::ZERO);
    fund_and_launder(&registry, &mut fixture, normal_capacity);

    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("laundered front should remain sabotageable")
        .commit(&mut fixture.state)
        .expect("sabotage should commit after laundering");
    let disrupted_gross =
        resolve_business_current_gross(&registry, &fixture.state, fixture.business)
            .expect("disrupted gross should resolve");
    let disrupted_capacity = crate::finance::helpers::resolve_basis_point_share(
        disrupted_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("disrupted laundering capacity should resolve");
    assert!(
        disrupted_capacity < normal_capacity,
        "fixture sabotage must reduce current laundering capacity"
    );
    assert!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("front economy should persist")
            .laundered_this_cycle()
            > disrupted_capacity,
        "prior valid laundering should now exceed only the later degraded capacity"
    );

    validate_state_against_registry(&registry, &fixture.state)
        .expect("later sabotage must not retroactively invalidate prior laundering");
    let envelope =
        build_save(&registry, &fixture.state).expect("post-sabotage laundering state should save");
    restore_save(&registry, envelope)
        .expect("post-sabotage laundering state should restore without rewriting history");
}

fn replace_serialized_economy(
    envelope: SaveEnvelope,
    original: &crate::economy::BusinessEconomyRecord,
    replacement: &BusinessEconomyRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("business economy should serialize");
    let mirror = business_economy_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("business economy mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement business economy should serialize");
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
        "serialized economy must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout business economy corruption must remain decodable")
}

#[derive(Clone, Serialize)]
struct BusinessCycleContextWire {
    business: BusinessId,
    business_version: u32,
    owner: BusinessOwner,
    occurred_at: SimTime,
}

#[test]
fn restore_rejects_future_loss_streak_anchor() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .expect("fixture economy should suspend")
        .commit(&mut fixture.state)
        .expect("fixture suspension should commit");
    validate_resume_business_economy(&registry, &fixture.state, fixture.business)
        .expect("fixture economy should resume")
        .commit(&mut fixture.state)
        .expect("fixture resumption should commit");

    let record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("resumed economy should persist");
    assert_eq!(record.loss_streak_anchor(), Some(fixture.state.now()));
    let mut corrupted = business_economy_wire(record);
    corrupted.loss_streak_anchor = Some(fixture.state.now() + SimDuration::ONE_MINUTE);
    let error = restore_save(
        &registry,
        replace_serialized_economy(
            build_save(&registry, &fixture.state)
                .expect("valid resumed economy should save before corruption"),
            record,
            &corrupted,
        ),
    )
    .expect_err("a future loss-streak anchor must fail the real restore boundary");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidBusinessEconomySchedule {
                business
            }
        ) if business == fixture.business
    ));
}

#[test]
fn restore_rejects_disruption_horizon_beyond_any_possible_current_hit() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("fixture disruption should validate")
        .commit(&mut fixture.state)
        .expect("fixture disruption should commit");

    let record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("disrupted economy should persist");
    let legitimate_horizon = resolve_business_disruption_horizon(
        fixture.state.now(),
        registry.business_disruption().duration(),
    );
    assert_eq!(record.disrupted_through(), Some(legitimate_horizon));
    let mut corrupted = business_economy_wire(record);
    corrupted.disrupted_through = Some(legitimate_horizon + SimDuration::ONE_MINUTE);
    let error = restore_save(
        &registry,
        replace_serialized_economy(
            build_save(&registry, &fixture.state)
                .expect("valid disrupted economy should save before corruption"),
            record,
            &corrupted,
        ),
    )
    .expect_err("an impossible future disruption horizon must fail the real restore boundary");
    assert!(matches!(
        error,
        LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidBusinessEconomySchedule {
                business
            }
        ) if business == fixture.business
    ));
}

#[derive(Clone, Serialize)]
struct BusinessCycleFinancialsWire {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    disrupted: bool,
}

#[derive(Clone, Serialize)]
struct BusinessCycleArtifactsWire {
    attention: AttentionClass,
    transaction: Option<crate::core::id::LedgerTransactionId>,
    information: Option<crate::core::id::InformationId>,
}

#[derive(Clone, Serialize)]
struct BusinessCycleRecordWire {
    id: BusinessCycleId,
    context: BusinessCycleContextWire,
    financials: BusinessCycleFinancialsWire,
    artifacts: BusinessCycleArtifactsWire,
}

fn business_cycle_wire(record: &crate::economy::BusinessCycleRecord) -> BusinessCycleRecordWire {
    BusinessCycleRecordWire {
        id: record.id(),
        context: BusinessCycleContextWire {
            business: record.business(),
            business_version: record.business_version(),
            owner: record.owner(),
            occurred_at: record.occurred_at(),
        },
        financials: BusinessCycleFinancialsWire {
            gross_revenue: record.gross_revenue(),
            operating_cost: record.operating_cost(),
            net_cash: record.net_cash(),
            variance_basis_points: record.variance_basis_points(),
            disrupted: record.disrupted(),
        },
        artifacts: BusinessCycleArtifactsWire {
            attention: record.attention(),
            transaction: record.transaction(),
            information: record.information(),
        },
    }
}

fn replace_serialized_cycle(
    envelope: SaveEnvelope,
    original: &crate::economy::BusinessCycleRecord,
    replacement: &BusinessCycleRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("business cycle should serialize");
    let mirror = business_cycle_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("business cycle mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement business cycle should serialize");
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
        .expect("same-layout business cycle corruption must remain decodable")
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_business_economy_fixture() -> BusinessEconomyFixture {
    make_business_economy_fixture_for_kind(OrganizationKind::Commercial)
}

fn make_business_economy_fixture_for_kind(kind: OrganizationKind) -> BusinessEconomyFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xB051_1932);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Legitimate Holdings".to_owned(),
            kind,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Commercial Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(55),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Market Street Grocer".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("business fixture should validate");
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    BusinessEconomyFixture {
        state,
        business,
        organization,
        operating,
        settlement,
    }
}

fn fund_and_launder(
    registry: &Registry,
    fixture: &mut BusinessEconomyFixture,
    amount: Money,
) -> crate::core::id::LedgerTransactionId {
    let owner = FinancialOwner::Organization(fixture.organization);
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("concealed reserve should validate");
    let street = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        },
    )
    .expect("street cash should validate");
    let accounted = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted funds should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Seed laundering test cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: amount
                        .checked_neg()
                        .expect("positive test amount must negate"),
                },
                LedgerPosting {
                    account: street,
                    amount,
                },
            ],
            authorization: None,
        },
    )
    .expect("test cash transfer should validate")
    .commit(&mut fixture.state)
    .expect("test cash transfer should commit");
    validate_launder_funds(
        registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: street,
            business: fixture.business,
            accounted_account: accounted,
            amount,
        },
    )
    .expect("test laundering should validate")
    .commit(&mut fixture.state)
    .expect("test laundering should commit")
}

fn establish_business_economy(registry: &Registry, fixture: &mut BusinessEconomyFixture) {
    validate_establish_business_economy(
        registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: fixture.operating,
            settlement_account: fixture.settlement,
        },
    )
    .expect("business economy fixture should validate")
    .commit(&mut fixture.state)
    .expect("business economy fixture should commit");
}

#[test]
fn establishment_rejects_cycle_schedule_beyond_clock_horizon_without_mutation() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    let cycle = registry
        .get_business(BusinessKind::Retail)
        .economics()
        .cycle();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(cycle.as_minutes()) + 1,
    ));
    let before = bincode::serialize(&fixture.state).expect("boundary fixture should serialize");

    let error = match validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: fixture.operating,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("unrepresentable business cycle schedule must reject during validation"),
        Err(error) => error,
    };
    assert_eq!(error, BusinessEconomyError::SimulationTimeOverflow);
    assert_eq!(
        bincode::serialize(&fixture.state).expect("rejected boundary state should serialize"),
        before,
        "failed schedule capacity must not establish or otherwise mutate the economy"
    );
}

#[test]
fn routine_business_cycle_records_causal_economics_and_balanced_settlement() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("due business cycle should resolve");
    // Assert derived invariants rather than hard-coded cents so content tuning does not
    // spuriously break the contract: cost is authored base, net is gross-cost, attention
    // follows notable threshold, and settlement is the ledger mirror.
    let business_kind = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("fixture business should exist")
        .kind();
    let economics = registry.get_business(business_kind).economics();
    let business = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("fixture business should exist");
    let neighborhood = fixture
        .state
        .world()
        .get_neighborhood(business.neighborhood())
        .expect("business neighborhood should exist");
    let expected_police = crate::finance::helpers::weighted_rating(
        economics.police_cost_per_point(),
        neighborhood.profile().institutions.police_presence.value(),
    )
    .expect("police cost should not overflow");
    let expected_cost = economics
        .base_operating_cost()
        .checked_add(expected_police)
        .expect("business cost should not overflow");
    assert_eq!(plan.economics.operating_cost, expected_cost);
    assert_eq!(
        plan.economics.net_cash,
        plan.economics
            .gross_revenue
            .checked_sub(plan.economics.operating_cost)
            .expect("net cash should be gross - cost")
    );
    assert_eq!(plan.economics.attention, AttentionClass::Routine);
    assert!(plan.economics.gross_revenue.cents() >= economics.base_gross().cents());

    let cycle = validate_business_cycle_plan(&fixture.state, plan)
        .expect("business cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("business cycle should commit");
    let cycle = fixture
        .state
        .economy()
        .get_cycle(cycle)
        .expect("business cycle should persist");
    assert!(cycle.transaction().is_some());
    assert!(cycle.information().is_none());
    let operating_balance = fixture
        .state
        .finance()
        .get_account(fixture.operating)
        .expect("operating account should exist")
        .balance();
    let settlement_balance = fixture
        .state
        .finance()
        .get_account(fixture.settlement)
        .expect("settlement account should exist")
        .balance();
    assert_eq!(operating_balance, cycle.net_cash());
    assert_eq!(
        settlement_balance,
        Money::from_cents(-operating_balance.cents())
    );
    validate_invariants(&fixture.state);
}

#[test]
fn ownership_change_invalidates_prevalidated_business_cycle_atomically() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
        .expect("due business cycle should resolve");
    let validated = validate_business_cycle_plan(&fixture.state, plan)
        .expect("business cycle should validate before ownership changes");
    let successor = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Successor Holdings".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("successor organization should validate");
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(successor),
    )
    .expect("business ownership change should validate")
    .commit(&mut fixture.state)
    .expect("business ownership change should commit");

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("ownership change must invalidate a prevalidated cycle");
    assert_eq!(
        error,
        BusinessEconomyError::StaleBusiness {
            business: fixture.business,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        fixture.state.economy().cycles_for(fixture.business).count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .expect("operating account should exist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .expect("settlement account should exist")
            .balance(),
        Money::ZERO
    );
    validate_invariants(&fixture.state);
}

#[test]
fn transferred_business_cycles_remain_attributed_to_the_owner_at_commit() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first_cycle = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
        .expect("first due business cycle should resolve");
    let first_cycle = validate_business_cycle_plan(&fixture.state, first_cycle)
        .expect("first business cycle should validate")
        .commit(&mut fixture.state)
        .expect("first business cycle should commit");
    let first_cycle_record = fixture
        .state
        .economy()
        .get_cycle(first_cycle)
        .expect("first business cycle should persist");
    assert_eq!(
        first_cycle_record.owner(),
        BusinessOwner::Organization(fixture.organization)
    );
    assert_eq!(first_cycle_record.business_version(), 1);

    let successor = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Acquiring Company".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("acquiring organization should validate");
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(successor),
    )
    .expect("same-minute ownership transfer should validate")
    .commit(&mut fixture.state)
    .expect("same-minute ownership transfer should commit");

    let transfer_minute = fixture.state.now();
    let original_boundary_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        fixture.organization,
        transfer_minute,
        transfer_minute,
    )
    .expect("outgoing owner boundary summary should preserve its committed cycle");
    assert_eq!(original_boundary_summary.totals.business_count, 1);
    assert_eq!(
        original_boundary_summary.totals.cycle_count, 1,
        "the cycle's persisted owner proves the outgoing organization settled before the same-minute transfer"
    );
    let successor_boundary_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        successor,
        transfer_minute,
        transfer_minute,
    )
    .expect("incoming owner boundary summary should resolve");
    assert_eq!(successor_boundary_summary.totals.business_count, 1);
    assert_eq!(successor_boundary_summary.totals.cycle_count, 0);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second_cycle = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
        .expect("second due business cycle should resolve");
    let second_cycle = validate_business_cycle_plan(&fixture.state, second_cycle)
        .expect("second business cycle should validate")
        .commit(&mut fixture.state)
        .expect("second business cycle should commit");
    let second_cycle_record = fixture
        .state
        .economy()
        .get_cycle(second_cycle)
        .expect("second business cycle should persist");
    assert_eq!(
        second_cycle_record.owner(),
        BusinessOwner::Organization(successor)
    );
    assert_eq!(second_cycle_record.business_version(), 2);

    let original_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        fixture.organization,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("original owner summary should preserve historical attribution");
    let successor_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        successor,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("successor summary should include only post-transfer cycles");
    assert_eq!(original_summary.totals.business_count, 1);
    assert_eq!(original_summary.totals.cycle_count, 1);
    assert_eq!(original_summary.totals.notable_cycle_count, 1);
    assert_eq!(successor_summary.totals.business_count, 1);
    assert_eq!(successor_summary.totals.cycle_count, 1);
    assert_eq!(successor_summary.totals.notable_cycle_count, 1);

    let original_report = validate_organization_financial_report(
        &fixture.state,
        fixture.organization,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("original owner report should retain its notable historical cycle")
    .commit(&mut fixture.state)
    .expect("original owner report should commit");
    let successor_report = validate_organization_financial_report(
        &fixture.state,
        successor,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("successor report should include its notable post-transfer cycle")
    .commit(&mut fixture.state)
    .expect("successor report should commit");
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(original_report)
            .expect("original owner report should persist")
            .entries()
            .len(),
        2
    );
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(successor_report)
            .expect("successor report should persist")
            .entries()
            .len(),
        2
    );
    validate_invariants(&fixture.state);
}

#[test]
fn establishment_and_resume_schedule_from_commit_time() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    let establishment = validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: fixture.operating,
            settlement_account: fixture.settlement,
        },
    )
    .expect("business economy should validate before delayed commit");
    fixture.state.advance_clock(SimDuration::from_minutes(60));
    establishment
        .commit(&mut fixture.state)
        .expect("delayed establishment should commit");
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("business economy should exist");
    assert_eq!(economy.established_at(), SimTime::from_minutes(60));
    assert_eq!(economy.next_cycle_at(), Some(SimTime::from_minutes(1_500)));

    validate_suspend_business_economy(&fixture.state, fixture.business)
        .expect("business economy should suspend")
        .commit(&mut fixture.state)
        .expect("business suspension should commit");
    let resume = validate_resume_business_economy(&registry, &fixture.state, fixture.business)
        .expect("business economy should validate for resume");
    fixture.state.advance_clock(SimDuration::from_minutes(30));
    resume
        .commit(&mut fixture.state)
        .expect("delayed business resume should commit");
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("business economy should still exist");
    assert_eq!(economy.next_cycle_at(), Some(SimTime::from_minutes(1_530)));
    validate_invariants(&fixture.state);
}

#[test]
fn notable_owned_business_cycle_creates_accounting_information_for_owner() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
        .expect("material business variance should resolve");
    assert_eq!(plan.economics.attention, AttentionClass::Notable);
    let cycle = validate_business_cycle_plan(&fixture.state, plan)
        .expect("material business cycle should validate")
        .commit(&mut fixture.state)
        .expect("material business cycle should commit");
    let cycle = fixture
        .state
        .economy()
        .get_cycle(cycle)
        .expect("cycle should persist");
    let information = cycle
        .information()
        .expect("owned notable business cycle should create accounting information");
    let information = fixture
        .state
        .intelligence()
        .get_information(information)
        .expect("accounting information should persist");
    assert_eq!(
        information.holder(),
        KnowledgeHolder::Organization(fixture.organization)
    );
    assert_eq!(information.source_kind(), InformationSourceKind::Accountant);
    assert_eq!(information.subject(), EntityRef::Business(fixture.business));

    let organization_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        fixture.organization,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("organization business summary should resolve");
    assert_eq!(organization_summary.totals.business_count, 1);
    assert_eq!(organization_summary.totals.cycle_count, 1);
    assert_eq!(organization_summary.totals.notable_cycle_count, 1);
    assert!(organization_summary.totals.gross_revenue > Money::ZERO);
    assert!(organization_summary.totals.operating_cost > Money::ZERO);
    assert_eq!(
        organization_summary.totals.net_cash,
        organization_summary
            .totals
            .gross_revenue
            .checked_sub(organization_summary.totals.operating_cost)
            .expect("summary net should equal gross less operating cost")
    );

    let report = validate_organization_financial_report(
        &fixture.state,
        fixture.organization,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("organization financial report should synthesize legitimate business history")
    .commit(&mut fixture.state)
    .expect("organization financial report should commit");
    let report = fixture
        .state
        .reports()
        .get_report(report)
        .expect("organization financial report should persist");
    assert_eq!(report.kind(), ReportKind::Financial);
    assert_eq!(report.entries().len(), 2);
    assert_eq!(report.entries()[0].attention, AttentionClass::Routine);
    assert_eq!(report.entries()[1].attention, AttentionClass::Notable);
    assert_eq!(report.entries()[1].sources.len(), 1);
    assert!(
        report.entries()[1]
            .entities
            .contains(&EntityRef::Business(fixture.business))
    );
    validate_invariants(&fixture.state);
}

#[test]
fn business_economy_is_unique_and_suspension_removes_due_work() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let duplicate = match validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: fixture.operating,
            settlement_account: fixture.settlement,
        },
    ) {
        Ok(_) => panic!("one business must not have multiple operating economy records"),
        Err(error) => error,
    };
    assert_eq!(
        duplicate,
        BusinessEconomyError::ExistingBusinessEconomy(fixture.business)
    );

    validate_suspend_business_economy(&fixture.state, fixture.business)
        .expect("active business economy should suspend")
        .commit(&mut fixture.state)
        .expect("business suspension should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert!(find_due_businesses(&fixture.state).is_empty());
    validate_invariants(&fixture.state);
}

#[test]
fn due_businesses_preserve_schedule_chronology_before_id_order() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    let later_due_lower_id = fixture.business;
    let neighborhood = fixture
        .state
        .world()
        .get_business(later_due_lower_id)
        .expect("first business should persist")
        .neighborhood();
    let earlier_due_higher_id = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Chronology Corner Store".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("second business should validate");
    let second_operating = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(earlier_due_higher_id),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("second operating account should validate");
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(earlier_due_higher_id),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second settlement account should validate");
    validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: earlier_due_higher_id,
            operating_account: second_operating,
            settlement_account: second_settlement,
        },
    )
    .expect("second business economy should validate")
    .commit(&mut fixture.state)
    .expect("second business economy should commit");
    assert!(later_due_lower_id < earlier_due_higher_id);

    // Both businesses started together. Rescheduling only the lower-ID economy makes it due
    // later, so a catch-up scan must preserve (due time, business ID) rather than flattening
    // everything back to creation order.
    validate_suspend_business_economy(&fixture.state, later_due_lower_id)
        .expect("lower-ID economy should suspend")
        .commit(&mut fixture.state)
        .expect("lower-ID economy suspension should commit");
    fixture.state.advance_clock(SimDuration::from_minutes(60));
    validate_resume_business_economy(&registry, &fixture.state, later_due_lower_id)
        .expect("lower-ID economy should resume")
        .commit(&mut fixture.state)
        .expect("lower-ID economy resume should commit");

    let earlier_due_at = fixture
        .state
        .economy()
        .get_business_economy(earlier_due_higher_id)
        .and_then(|record| record.next_cycle_at())
        .expect("higher-ID economy should remain scheduled");
    let later_due_at = fixture
        .state
        .economy()
        .get_business_economy(later_due_lower_id)
        .and_then(|record| record.next_cycle_at())
        .expect("resumed lower-ID economy should be scheduled");
    assert!(earlier_due_at < later_due_at);
    let catch_up_minutes =
        u32::try_from(later_due_at.as_minutes() - fixture.state.now().as_minutes())
            .expect("fixture catch-up duration must fit SimDuration");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(catch_up_minutes));

    assert_eq!(
        find_due_businesses(&fixture.state),
        vec![earlier_due_higher_id, later_due_lower_id],
        "older overdue business work must settle before a later-due lower ID"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn save_round_trip_preserves_business_schedule_and_deterministic_tick_resolution() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    let successor = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Saved Successor Holdings".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("saved successor organization should validate");
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(successor),
    )
    .expect("pre-save ownership transfer should validate")
    .commit(&mut fixture.state)
    .expect("pre-save ownership transfer should commit");

    let envelope = build_save(&registry, &fixture.state)
        .expect("business economy state should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored =
        restore_save(&registry, decoded).expect("business economy save should restore");

    let original = run_tick(&registry, &mut fixture.state);
    let continued = run_tick(&registry, &mut restored);
    assert_eq!(original, continued);
    assert_eq!(original.business_cycles.len(), 1);
    let original_cycle = fixture
        .state
        .economy()
        .get_cycle(original.business_cycles[0])
        .expect("original cycle should exist");
    let restored_cycle = restored
        .economy()
        .get_cycle(continued.business_cycles[0])
        .expect("restored cycle should exist");
    assert_eq!(
        original_cycle.owner(),
        BusinessOwner::Organization(successor)
    );
    assert_eq!(
        restored_cycle.owner(),
        BusinessOwner::Organization(successor)
    );
    assert_eq!(original_cycle.business_version(), 2);
    assert_eq!(restored_cycle.business_version(), 2);
    assert_eq!(
        restored
            .world()
            .business_ownership_history(fixture.business)
            .count(),
        2
    );
    assert_eq!(
        original_cycle.gross_revenue(),
        restored_cycle.gross_revenue()
    );
    assert_eq!(original_cycle.net_cash(), restored_cycle.net_cash());
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .expect("original operating account should exist")
            .balance(),
        restored
            .finance()
            .get_account(fixture.operating)
            .expect("restored operating account should exist")
            .balance()
    );
    validate_invariants(&fixture.state);
    validate_invariants(&restored);
}

#[test]
fn later_sabotage_does_not_rewrite_prior_cycle_disruption_history() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("normal due cycle should decide");
    assert_eq!(plan.economics.attention, AttentionClass::Routine);
    let cycle = validate_business_cycle_plan(&fixture.state, plan)
        .expect("normal cycle should validate")
        .commit(&mut fixture.state)
        .expect("normal cycle should commit");
    assert!(
        !fixture
            .state
            .economy()
            .get_cycle(cycle)
            .expect("normal cycle should persist")
            .disrupted()
    );

    fixture.state.advance_clock(SimDuration::from_minutes(60));
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("later sabotage should validate")
        .commit(&mut fixture.state)
        .expect("later sabotage should commit");

    let prior = fixture
        .state
        .economy()
        .get_cycle(cycle)
        .expect("prior cycle should persist");
    assert!(!prior.disrupted());
    assert_eq!(prior.attention(), AttentionClass::Routine);
    crate::core::invariants::validate_state_against_registry(&registry, &fixture.state)
        .expect("later sabotage must not retroactively invalidate an earlier normal cycle");
    build_save(&registry, &fixture.state)
        .expect("state with later sabotage and prior normal cycle should remain save-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn registry_validation_rejects_internally_balanced_unauthored_cycle_financials() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let cycle = validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
            .expect("due business cycle should decide"),
    )
    .expect("business cycle should validate")
    .commit(&mut fixture.state)
    .expect("business cycle should commit");

    // Preserve net and therefore preserve the exact ledger postings while making gross and
    // operating cost equally too large. Corrupt the persisted wire representation rather than
    // bypassing the economy owner: the real load boundary must reject economics that are
    // internally balanced but impossible under authored content.
    let delta = Money::from_cents(100);
    let record = fixture
        .state
        .economy()
        .get_cycle(cycle)
        .expect("cycle fixture should persist");
    let mut corrupted = business_cycle_wire(record);
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
        .expect("valid business cycle should save before corruption");
    let corrupted_envelope = replace_serialized_cycle(envelope, record, &corrupted);
    let error = restore_save(&registry, corrupted_envelope)
        .expect_err("unauthored business economics must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidBusinessCycle {
                    cycle: invalid,
                }
            ) if invalid == cycle
        ),
        "expected invalid business cycle, got {error:?}"
    );
}

#[test]
fn sabotage_disruption_degrades_cycle_gross_until_the_horizon_passes() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let disruption = validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("disruption should validate against an active economy");
    let horizon = resolve_business_disruption_horizon(
        fixture.state.now(),
        registry.business_disruption().duration(),
    );
    disruption
        .commit(&mut fixture.state)
        .expect("disruption should commit");
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("disrupted economy should exist");
    assert_eq!(economy.disrupted_through(), Some(horizon));
    assert!(economy.is_disrupted(fixture.state.now()));
    assert!(
        economy.is_disrupted(horizon),
        "inclusive horizon must remain the final affected minute"
    );
    assert!(
        !economy.is_disrupted(horizon + SimDuration::ONE_MINUTE),
        "authored duration must not leak into an extra endpoint minute"
    );

    // The undisrupted gross for this neighborhood profile, computed through the same
    // production math so content tuning cannot break the contract spuriously.
    let business = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("fixture business should exist");
    let neighborhood = fixture
        .state
        .world()
        .get_neighborhood(business.neighborhood())
        .expect("business neighborhood should exist");
    let normal_gross = resolve_gross_before_variance(
        fixture.business,
        registry.get_business(business.kind()).economics(),
        neighborhood.profile(),
    )
    .expect("normal gross should resolve");
    let disrupted_basis_points = registry.business_disruption().gross_basis_points();
    let expected_disrupted_gross = Money::from_cents(
        (i128::from(normal_gross.cents()) * i128::from(disrupted_basis_points) / 10_000)
            .try_into()
            .expect("disrupted gross should fit money"),
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let disrupted_plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("due cycle inside the disruption horizon should settle degraded");
    assert_eq!(
        disrupted_plan.economics.gross_revenue,
        expected_disrupted_gross
    );
    validate_business_cycle_plan(&fixture.state, disrupted_plan)
        .expect("disrupted cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("disrupted cycle should commit");

    // The next daily settlement is exactly one minute after the inclusive disruption horizon.
    // It must recover rather than suffering a third degraded cycle from an off-by-one endpoint.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let recovered_plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("due cycle immediately after the horizon should recover");
    assert_eq!(
        recovered_plan.economics.gross_revenue, normal_gross,
        "cycle immediately after the horizon must earn undisrupted gross"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn repeated_sabotage_extends_but_never_shortens_the_disruption_horizon() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let first_horizon = resolve_business_disruption_horizon(
        fixture.state.now(),
        registry.business_disruption().duration(),
    );
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("first disruption should validate")
        .commit(&mut fixture.state)
        .expect("first disruption should commit");

    // A second hit inside the first horizon pushes the horizon later from the new instant.
    fixture.state.advance_clock(SimDuration::from_minutes(600));
    let second_horizon = resolve_business_disruption_horizon(
        fixture.state.now(),
        registry.business_disruption().duration(),
    );
    assert!(second_horizon > first_horizon);
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("second disruption should validate")
        .commit(&mut fixture.state)
        .expect("second disruption should commit");
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("disrupted economy should exist")
            .disrupted_through(),
        Some(second_horizon),
        "a later hit must extend the horizon"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn same_minute_repeated_sabotage_keeps_success_without_redundant_economy_version_bump() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("first same-minute disruption should validate")
        .commit(&mut fixture.state)
        .expect("first same-minute disruption should commit");
    let after_first = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("disrupted economy should persist");
    let first_version = after_first.version();
    let first_horizon = after_first.disrupted_through();

    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("a second successful sabotage at the same instant remains a valid consequence")
        .commit(&mut fixture.state)
        .expect("redundant horizon application should succeed without mutating the economy");
    let after_second = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("business economy should persist");
    assert_eq!(after_second.disrupted_through(), first_horizon);
    assert_eq!(
        after_second.version(),
        first_version,
        "an equal sabotage horizon must not invalidate unrelated economy snapshots"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn old_disruption_remains_registry_valid_when_current_clock_nears_horizon() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("ordinary disruption should validate")
        .commit(&mut fixture.state)
        .expect("ordinary disruption should commit");
    let duration = registry.business_disruption().duration();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(duration.as_minutes()) + 1,
    ));

    validate_state_against_registry(&registry, &fixture.state).expect(
        "an old representable disruption must stay valid after the clock moves near its horizon",
    );
    validate_invariants(&fixture.state);
}

#[test]
fn disruption_near_clock_horizon_clamps_instead_of_rejecting_success() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    let duration = registry.business_disruption().duration();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(duration.as_minutes()) + 1,
    ));

    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect(
            "successful sabotage should remain representable through the final simulation minute",
        )
        .commit(&mut fixture.state)
        .expect("clamped disruption should commit");
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("disrupted business economy should persist")
            .disrupted_through(),
        Some(SimTime::from_minutes(u64::MAX))
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("clamped disruption horizon should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn newly_established_business_near_horizon_can_persist_clamped_disruption() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    let cycle = registry
        .get_business(BusinessKind::Retail)
        .economics()
        .cycle();
    let established_at = SimTime::from_minutes(u64::MAX - u64::from(cycle.as_minutes()));
    fixture.state.set_now_for_test(established_at);
    establish_business_economy(&registry, &mut fixture);
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("near-horizon economy should establish")
            .next_cycle_at(),
        Some(SimTime::from_minutes(u64::MAX))
    );

    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("a newly established near-horizon business should still be disruptable")
        .commit(&mut fixture.state)
        .expect("clamped disruption should commit on the newly established economy");
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("disrupted near-horizon economy should persist")
            .disrupted_through(),
        Some(SimTime::from_minutes(u64::MAX))
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("registry validation must use the same clamped earliest disruption horizon");
    validate_invariants(&fixture.state);

    restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("new near-horizon disruption should remain saveable"),
    )
    .expect("new near-horizon disruption should survive restore");
}

#[test]
fn stale_economy_version_rejects_sabotage_disruption_atomically() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let disruption = validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("disruption should validate");
    // Mutate the economy record between validation and commit (status flip-flop bumps version).
    let business = fixture.business;
    let next_cycle_at = fixture
        .state
        .economy()
        .get_business_economy(business)
        .expect("economy should exist")
        .next_cycle_at();
    fixture
        .state
        .economy
        .set_status(business, BusinessOperatingStatus::Suspended, None, None);
    if let Some(next_cycle_at) = next_cycle_at {
        fixture.state.economy.set_status(
            business,
            BusinessOperatingStatus::Active,
            Some(next_cycle_at),
            None,
        );
    }
    let error = disruption
        .commit(&mut fixture.state)
        .expect_err("stale validated disruption must be rejected");
    assert!(matches!(error, BusinessEconomyError::StaleEconomy { .. }));
}

#[test]
fn chronic_losing_business_surfaces_losses_then_suspends_at_the_authored_threshold() {
    let registry = build_registry();
    let mut state = AppState::new(0x5EED_5105);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Losing Holdings".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Depressed Ward".to_owned(),
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
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Bleeding Warehouse".to_owned(),
            kind: BusinessKind::Warehouse,
            functions: BTreeSet::from([BusinessFunction::Warehousing]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("business fixture should validate");
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    crate::economy::business_economy_system::validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("business economy should establish")
    .commit(&mut state)
    .expect("business economy should commit");

    let threshold = registry
        .get_business(BusinessKind::Warehouse)
        .economics()
        .losing_cycles_before_suspension() as usize;
    let mut last_information = None;
    for cycle_index in 0..threshold {
        state.advance_clock(SimDuration::from_minutes(1_440));
        // Maximum authored downside variance keeps every settlement net-negative.
        let plan = decide_business_cycle(&registry, &state, business, -500)
            .expect("losing business cycle should decide");
        assert!(
            plan.economics.net_cash.cents() < 0,
            "fixture must produce a losing settlement"
        );
        assert_eq!(plan.economics.attention, AttentionClass::Notable);
        let cycle = validate_business_cycle_plan(&state, plan)
            .expect("losing cycle plan should validate")
            .commit(&mut state)
            .expect("losing cycle should commit");
        let record = state
            .economy()
            .get_cycle(cycle)
            .expect("cycle record should persist");
        last_information = record.information();
        if cycle_index + 1 < threshold {
            assert_eq!(
                state
                    .economy()
                    .get_business_economy(business)
                    .map(|economy| economy.status()),
                Some(crate::economy::BusinessOperatingStatus::Active),
                "economy stays active below the suspension threshold"
            );
        }
    }
    let economy = state
        .economy()
        .get_business_economy(business)
        .expect("economy should persist");
    assert_eq!(
        economy.status(),
        crate::economy::BusinessOperatingStatus::Suspended
    );
    assert_eq!(economy.next_cycle_at(), None);
    // The accountant report names the consequence so leadership can act on it.
    let information = state
        .intelligence()
        .get_information(last_information.expect("notable losing cycle should report"))
        .expect("accountant information should persist");
    assert!(information.summary().contains("suspended"));

    // No further cycles fire while suspended; resumption is the manual canonical path.
    state.advance_clock(SimDuration::from_minutes(1_440));
    assert!(state.economy().due_at_or_before(state.now()).is_empty());
    crate::economy::business_economy_system::validate_resume_business_economy(
        &registry, &state, business,
    )
    .expect("suspended economy should resume")
    .commit(&mut state)
    .expect("resumed economy should commit");
    assert_eq!(
        state
            .economy()
            .get_business_economy(business)
            .map(|economy| economy.status()),
        Some(crate::economy::BusinessOperatingStatus::Active)
    );

    // Resumption restarts the losing-cycle grace window: pre-suspension losses must not
    // re-suspend the economy on its first post-resume losing settlement.
    for cycle_index in 0..threshold {
        state.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_business_cycle(&registry, &state, business, -500)
            .expect("post-resume losing cycle should decide");
        assert!(plan.economics.net_cash.cents() < 0);
        validate_business_cycle_plan(&state, plan)
            .expect("post-resume losing plan should validate")
            .commit(&mut state)
            .expect("post-resume losing cycle should commit");
        let status = state
            .economy()
            .get_business_economy(business)
            .map(|economy| economy.status());
        if cycle_index + 1 < threshold {
            assert_eq!(
                status,
                Some(crate::economy::BusinessOperatingStatus::Active),
                "a resumed economy gets a fresh grace window"
            );
        } else {
            assert_eq!(
                status,
                Some(crate::economy::BusinessOperatingStatus::Suspended),
                "threshold consecutive post-resume losses suspend again"
            );
        }
    }
    crate::core::invariants::validate_invariants(&state);
}
