//! Focused tests for business establishment, ownership transfer, cycle settlement, and reporting inputs.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_reporting::{
    resolve_business_financial_summary, resolve_organization_business_financial_summary,
};
use crate::finance::finance_system::insert_account;
use crate::finance::{FinancialAccountDraft, FinancialOwner};
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
use std::collections::BTreeSet;

struct BusinessEconomyFixture {
    state: AppState,
    business: BusinessId,
    organization: crate::core::id::OrganizationId,
    operating: FinancialAccountId,
    settlement: FinancialAccountId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_business_economy_fixture() -> BusinessEconomyFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xB051_1932);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Legitimate Holdings".to_owned(),
            kind: OrganizationKind::Commercial,
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
    assert_eq!(plan.operating_cost(), expected_cost);
    assert_eq!(
        plan.net_cash(),
        plan.gross_revenue()
            .checked_sub(plan.operating_cost())
            .expect("net cash should be gross - cost")
    );
    assert_eq!(plan.attention(), AttentionClass::Routine);
    assert!(plan.gross_revenue().cents() >= economics.base_gross().cents());

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
    assert_eq!(plan.attention(), AttentionClass::Notable);
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

    let business_summary = resolve_business_financial_summary(
        &fixture.state,
        fixture.business,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("business financial summary should resolve");
    let organization_summary = resolve_organization_business_financial_summary(
        &fixture.state,
        fixture.organization,
        SimTime::ZERO,
        fixture.state.now(),
    )
    .expect("organization business summary should resolve");
    assert_eq!(business_summary.totals, organization_summary.totals);
    assert_eq!(business_summary.totals.notable_cycle_count, 1);

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
fn sabotage_disruption_degrades_cycle_gross_until_the_horizon_passes() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let disruption = validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("disruption should validate against an active economy");
    let horizon = fixture.state.now() + registry.business_disruption().duration();
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
    assert_eq!(disrupted_plan.gross_revenue(), expected_disrupted_gross);
    validate_business_cycle_plan(&fixture.state, disrupted_plan)
        .expect("disrupted cycle plan should validate")
        .commit(&mut fixture.state)
        .expect("disrupted cycle should commit");

    // After the horizon passes the same business earns normal gross again.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_441));
    let recovered_plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("due cycle after the horizon should recover");
    assert_eq!(
        recovered_plan.gross_revenue(),
        normal_gross,
        "cycle after the horizon must earn undisrupted gross"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn repeated_sabotage_extends_but_never_shortens_the_disruption_horizon() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);

    let first_horizon = fixture.state.now() + registry.business_disruption().duration();
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("first disruption should validate")
        .commit(&mut fixture.state)
        .expect("first disruption should commit");

    // A second hit inside the first horizon pushes the horizon later from the new instant.
    fixture.state.advance_clock(SimDuration::from_minutes(600));
    let second_horizon = fixture.state.now() + registry.business_disruption().duration();
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
            plan.net_cash().cents() < 0,
            "fixture must produce a losing settlement"
        );
        assert_eq!(plan.attention(), AttentionClass::Notable);
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
        assert!(plan.net_cash().cents() < 0);
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
