//! Business acquisition ownership, finance, and lifecycle integration tests.

use super::*;
use crate::build_registry;
use crate::core::id::IdKind;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::time::SimDuration;
use crate::finance::finance_system::insert_account;
use crate::world::world_system::{insert_business, insert_neighborhood, insert_organization};
use crate::world::{
    BusinessDraft, BusinessFunction, BusinessKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    Rating,
};
use std::collections::BTreeSet;

const SEED: u64 = 0x0AC0_5171;

struct AcquisitionFixture {
    registry: Registry,
    state: AppState,
    organization: OrganizationId,
    business: BusinessId,
    accounted: FinancialAccountId,
    street: FinancialAccountId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn make_independent_fixture() -> AcquisitionFixture {
    let registry = build_registry();
    let mut state = AppState::new(SEED);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Marlowe Holdings".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Harbor Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(55),
                    commercial_activity: rating(60),
                    illicit_demand: rating(40),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(30),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Pier Nine Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business fixture should validate");
    let accounted = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted-funds fixture should validate");
    let street = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("street-cash fixture should validate");
    AcquisitionFixture {
        registry,
        state,
        organization,
        business,
        accounted,
        street,
    }
}

fn hospitality_price(fixture: &AcquisitionFixture) -> Money {
    fixture
        .registry
        .get_business(BusinessKind::Hospitality)
        .economics()
        .acquisition_cost()
}

fn fund_accounted_from_street(fixture: &mut AcquisitionFixture, cents: i64) {
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fixture capitalization".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: fixture.accounted,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )
    .expect("fixture capitalization should validate")
    .commit(&mut fixture.state)
    .expect("fixture capitalization should commit");
}

fn acquisition_draft(fixture: &AcquisitionFixture) -> BusinessAcquisitionDraft {
    BusinessAcquisitionDraft {
        organization: fixture.organization,
        business: fixture.business,
        funding_accounts: BTreeSet::from([fixture.accounted]),
    }
}

fn establish_existing_independent_economy(
    fixture: &mut AcquisitionFixture,
) -> (FinancialAccountId, FinancialAccountId) {
    let operating = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(fixture.business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("existing operating account should validate");
    let settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(fixture.business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("existing settlement account should validate");
    business_economy_system::validate_establish_business_economy(
        &fixture.registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("independent economy should validate")
    .commit(&mut fixture.state)
    .expect("independent economy should commit");
    (operating, settlement)
}

#[test]
fn acquisition_buys_an_independent_business_without_returning_price_to_operating_cash() {
    let mut fixture = make_independent_fixture();
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());

    let acquired = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect("a funded independent acquisition must validate")
    .commit(&mut fixture.state)
    .expect("a validated acquisition must commit");

    assert_eq!(acquired.business, fixture.business);
    assert_eq!(acquired.price, price);
    assert!(acquired.established_economy);

    // Ownership moved through the canonical record.
    let business = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("acquired business must persist");
    assert_eq!(
        business.owner(),
        BusinessOwner::Organization(fixture.organization)
    );

    // The price leaves the buyer's spendable control. Fresh operating books begin at zero;
    // the non-liquid settlement account carries the external seller counterparty, and
    // accounted funds dropped by exactly the authored price.
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("an acquisition of an unoperated business must open its economy");
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(economy.operating_account())
            .expect("operating account must persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(economy.settlement_account())
            .expect("seller settlement account must persist")
            .balance(),
        price
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("funding account must persist")
            .balance(),
        Money::ZERO
    );

    // The purchase surfaces as player-visible financial information.
    let report = fixture
        .state
        .reports()
        .reports_for(fixture.organization)
        .find(|report| report.kind() == ReportKind::Financial)
        .expect("the acquisition must surface as a financial report");
    assert_eq!(report.entries()[0].attention, AttentionClass::Notable);
    assert!(
        report.entries()[0]
            .summary
            .contains("Pier Nine Social Club")
    );

    validate_invariants(&fixture.state);
}

#[test]
fn acquisition_reanchors_an_active_sellers_cycle_to_the_purchase_time() {
    let mut fixture = make_independent_fixture();
    establish_existing_independent_economy(&mut fixture);
    let cycle_duration = fixture
        .registry
        .get_business(BusinessKind::Hospitality)
        .economics()
        .cycle();
    let seller_due_at = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .and_then(|economy| economy.next_cycle_at())
        .expect("active seller economy should have a due time");
    fixture.state.advance_clock(SimDuration::from_minutes(
        cycle_duration
            .as_minutes()
            .checked_sub(1)
            .expect("positive cycle leaves a minute before settlement"),
    ));
    assert_eq!(
        seller_due_at,
        fixture.state.now() + SimDuration::ONE_MINUTE,
        "fixture must buy immediately before the seller's old settlement"
    );
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());

    validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect("active independent business should remain purchasable")
    .commit(&mut fixture.state)
    .expect("active-business acquisition should commit");

    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("buyer should retain the existing books");
    assert_eq!(
        economy.status(),
        crate::economy::BusinessOperatingStatus::Active
    );
    assert_eq!(economy.loss_streak_anchor(), Some(fixture.state.now()));
    assert_eq!(
        economy.next_cycle_at(),
        Some(fixture.state.now() + cycle_duration),
        "the buyer earns a fresh full cycle instead of inheriting the seller's nearly due one"
    );
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    assert!(
        fixture
            .state
            .economy()
            .due_at_or_before(fixture.state.now())
            .is_empty(),
        "the seller's pre-purchase settlement instant must no longer trigger for the buyer"
    );
    validate_state(&fixture.state).expect("reanchored acquisition should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn acquisition_aggregates_accounted_funds_across_multiple_accounts() {
    let mut fixture = make_independent_fixture();
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());
    let second_accounted = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("second accounted reserve should validate");
    let second_share = price.cents() / 2;
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Split acquisition reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.accounted,
                    amount: Money::from_cents(-second_share),
                },
                LedgerPosting {
                    account: second_accounted,
                    amount: Money::from_cents(second_share),
                },
            ],
            authorization: None,
        },
    )
    .expect("reserve split should validate")
    .commit(&mut fixture.state)
    .expect("reserve split should commit");
    assert!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("first accounted reserve should persist")
            .balance()
            < price
    );
    assert!(
        fixture
            .state
            .finance()
            .get_account(second_accounted)
            .expect("second accounted reserve should persist")
            .balance()
            < price
    );

    let acquired = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: fixture.organization,
            business: fixture.business,
            funding_accounts: BTreeSet::from([fixture.accounted, second_accounted]),
        },
    )
    .expect("aggregate accounted funds should cover the acquisition")
    .commit(&mut fixture.state)
    .expect("aggregate-funded acquisition should commit");

    assert_eq!(acquired.price, price);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("first accounted reserve should persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(second_accounted)
            .expect("second accounted reserve should persist")
            .balance(),
        Money::ZERO
    );
    validate_state(&fixture.state).expect("aggregate-funded acquisition should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn acquisition_id_preflight_rejects_without_opening_books_or_consuming_account_ids() {
    let mut fixture = make_independent_fixture();
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());
    let token = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect("funded acquisition should validate");
    let financial_next = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let account_count = fixture.state.finance().accounts().count();

    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Report, u32::MAX);
    let error = token
        .commit(&mut fixture.state)
        .expect_err("exhausted report ids must reject before any acquisition mutation");
    assert!(matches!(
        error,
        BusinessAcquisitionError::IdExhaustion(IdExhaustionError::Exhausted { kind: "report", .. })
    ));
    assert_eq!(
        fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("business must persist")
            .owner(),
        BusinessOwner::Independent
    );
    assert!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .is_none()
    );
    assert_eq!(fixture.state.finance().accounts().count(), account_count);
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        financial_next,
        "failed composite preflight must not consume planned account ids"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn short_accounted_funds_reject_the_purchase_without_touching_state() {
    let mut fixture = make_independent_fixture();
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents() - 1);

    let error = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect_err("a short purchase must reject");

    assert_eq!(
        error,
        BusinessAcquisitionError::InsufficientFunds {
            available_cents: price.cents() - 1,
            price_cents: price.cents(),
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("business must persist")
            .owner(),
        BusinessOwner::Independent,
        "a rejected acquisition leaves ownership untouched"
    );
    assert!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .is_none(),
        "a rejected acquisition opens no economy"
    );
    validate_invariants(&fixture.state);
}

/// A validated token held across other spending must not overdraw accounted funds:
/// commit re-checks solvency and rejects without touching ownership or books.
#[test]
fn stale_token_rejects_when_funding_drains_before_commit() {
    let mut fixture = make_independent_fixture();
    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());

    let token = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect("a funded acquisition must validate");

    // Another validated spend drains the funding account after validation.
    let drain = validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Interleaved spend".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.accounted,
                    amount: Money::from_cents(-price.cents()),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(price.cents()),
                },
            ],
            authorization: None,
        },
    )
    .expect("drain transfer should validate")
    .commit(&mut fixture.state)
    .expect("drain transfer should commit");
    let _ = drain;

    let error = token
        .commit(&mut fixture.state)
        .expect_err("a stale token whose funding drained must reject at commit");
    assert_eq!(
        error,
        BusinessAcquisitionError::InsufficientFunds {
            available_cents: 0,
            price_cents: price.cents(),
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("business must persist")
            .owner(),
        BusinessOwner::Independent,
        "the rejected stale purchase leaves ownership untouched"
    );
    assert!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .is_none(),
        "the rejected stale purchase opens no economy"
    );
    assert!(
        fixture
            .state
            .reports()
            .reports_for(fixture.organization)
            .find(|report| report.title() == "Business acquisition")
            .is_none(),
        "the rejected stale purchase publishes no report"
    );
    // The interleaved transfer restored both accounts to zero; nothing was minted or
    // lost anywhere in the failed purchase.
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.street)
            .expect("street account must persist")
            .balance(),
        Money::ZERO
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("accounted account must persist")
            .balance(),
        Money::ZERO
    );
    validate_invariants(&fixture.state);
}

#[test]
fn dirty_street_cash_cannot_buy_a_legitimate_business() {
    let fixture = make_independent_fixture();

    let error = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            funding_accounts: BTreeSet::from([fixture.street]),
            ..acquisition_draft(&fixture)
        },
    )
    .expect_err("dirty money must not buy legitimacy");

    assert_eq!(
        error,
        BusinessAcquisitionError::InvalidFundingAccountKind(fixture.street)
    );
    assert!(
        fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("business must persist")
            .owner()
            == BusinessOwner::Independent,
        "a rejected acquisition leaves ownership untouched"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn foreign_owned_targets_are_not_purchasable() {
    let mut fixture = make_independent_fixture();
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Rosetti Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival fixture should validate");
    crate::world::world_system::validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(rival),
    )
    .expect("rival ownership fixture should validate")
    .commit(&mut fixture.state)
    .expect("rival ownership fixture should commit");

    let error = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect_err("a rival-owned venue is not on the market");

    assert_eq!(
        error,
        BusinessAcquisitionError::NotIndependentlyOwned {
            business: fixture.business,
            owner: BusinessOwner::Organization(rival),
        }
    );
    validate_invariants(&fixture.state);
}

/// A chronic-loss suspension is automatic and resumption an owner decision — but an
/// independent business has no owner to decide. The acquisition path must therefore be
/// able to buy suspended books and reopen them, or sabotaged capital would stay dead
/// and unpurchasable forever.
#[test]
fn acquisition_buys_suspended_books_and_reopens_them_under_new_ownership() {
    use crate::economy::BusinessOperatingStatus;
    use crate::economy::business_economy_system::{
        validate_establish_business_economy, validate_suspend_business_economy,
    };

    let mut fixture = make_independent_fixture();

    // Give the target operating history: books over business-owned accounts, then the
    // canonical manual suspension (the same state chronic losses produce).
    let operating = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(fixture.business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account fixture should validate");
    let settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(fixture.business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account fixture should validate");
    validate_establish_business_economy(
        &fixture.registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("books should establish")
    .commit(&mut fixture.state)
    .expect("books should commit");
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .expect("active books should suspend")
        .commit(&mut fixture.state)
        .expect("suspension should commit");

    let price = hospitality_price(&fixture);
    fund_accounted_from_street(&mut fixture, price.cents());
    let acquired = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
    )
    .expect("a funded acquisition of suspended books must validate")
    .commit(&mut fixture.state)
    .expect("the acquisition must commit");

    assert!(
        !acquired.established_economy,
        "existing books are adopted, not re-established"
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("business must persist")
            .owner(),
        BusinessOwner::Organization(fixture.organization),
    );
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("acquired books must persist");
    assert_eq!(economy.status(), BusinessOperatingStatus::Active);
    // The purchase reopens the earning cycle under new ownership.
    let cycle_duration = fixture
        .registry
        .get_business(BusinessKind::Hospitality)
        .economics()
        .cycle();
    assert_eq!(
        economy.next_cycle_at(),
        Some(fixture.state.now() + cycle_duration)
    );
    // Purchase consideration must not refill a chronically losing business. Its operating
    // balance is preserved while the outside-seller counterparty receives the price.
    let operating_balance = fixture
        .state
        .finance()
        .get_account(operating)
        .expect("operating account must persist")
        .balance();
    assert_eq!(operating_balance, Money::ZERO);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(settlement)
            .expect("settlement account must persist")
            .balance(),
        price
    );
    // Ownership moved, so the same acquisition cannot run twice.
    assert!(matches!(
        validate_acquire_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture)
        ),
        Err(BusinessAcquisitionError::NotIndependentlyOwned { .. })
    ));
    validate_state(&fixture.state).expect("acquired state should remain structurally valid");
    validate_invariants(&fixture.state);
}
