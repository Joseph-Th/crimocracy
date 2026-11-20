//! Strategic investment: expansion beyond local inventory and genuine seller consent.

use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

fn executive(fixture: &mut AcquisitionFixture, organization: OrganizationId) {
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: format!("Executive {organization}"),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
}

fn another_property(
    fixture: &mut AcquisitionFixture,
    owner: BusinessOwner,
    new_district: bool,
) -> BusinessId {
    let original = fixture.business;
    let neighborhood = fixture
        .state
        .world()
        .get_business(original)
        .unwrap()
        .neighborhood();
    let neighborhood = if new_district {
        let profile = fixture
            .state
            .world()
            .get_neighborhood(neighborhood)
            .unwrap()
            .profile();
        insert_neighborhood(
            &mut fixture.state,
            NeighborhoodDraft {
                name: "New Commercial Ward".into(),
                profile,
            },
        )
        .unwrap()
    } else {
        neighborhood
    };
    let id = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Additional Storefront".into(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([BusinessFunction::CashIntensive]),
            neighborhood,
            owner,
        },
    )
    .unwrap();
    fixture.business = id;
    establish_existing_independent_economy(fixture);
    fixture.business = original;
    id
}

fn organization(
    fixture: &mut AcquisitionFixture,
    name: &str,
    kind: OrganizationKind,
) -> OrganizationId {
    insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: name.into(),
            kind,
        },
    )
    .unwrap()
}

fn owner(fixture: &AcquisitionFixture, business: BusinessId) -> BusinessOwner {
    fixture
        .state
        .world()
        .get_business(business)
        .unwrap()
        .owner()
}

#[test]
fn rival_expands_beyond_its_existing_district_after_local_inventory_sells() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    let player = organization(&mut fixture, "Player", OrganizationKind::Criminal);
    designate_player_organization(&mut fixture.state, player).unwrap();
    executive(&mut fixture, rival);
    establish_existing_independent_economy(&mut fixture);
    let remote = another_property(&mut fixture, BusinessOwner::Independent, true);
    let local = fixture.business;
    another_property(&mut fixture, BusinessOwner::Organization(rival), false);
    let price = hospitality_price(&fixture).cents();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price * 2 + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * 1440));
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![local]
    );
    assert_eq!(owner(&fixture, remote), BusinessOwner::Independent);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * 1440));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![remote]
    );
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut restored),
        vec![remote]
    );
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    assert_eq!(owner(&fixture, remote), BusinessOwner::Organization(rival));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .unwrap()
            .balance()
            .cents(),
        reserve
    );
}

#[test]
fn rival_negotiates_after_listings_end_without_unconsented_player_sales() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    let player = organization(&mut fixture, "Player", OrganizationKind::Criminal);
    let merchant = organization(
        &mut fixture,
        "Commercial seller",
        OrganizationKind::Commercial,
    );
    designate_player_organization(&mut fixture.state, player).unwrap();
    executive(&mut fixture, rival);
    establish_existing_independent_economy(&mut fixture);
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(merchant),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let player_shop = another_property(&mut fixture, BusinessOwner::Organization(player), false);
    let purchase_price = i64::try_from(
        negotiated_reference_price(&fixture.registry, &fixture.state, fixture.business).unwrap(),
    )
    .unwrap();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, purchase_price * 3 + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * 1440));
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![fixture.business]
    );
    assert_eq!(
        owner(&fixture, fixture.business),
        BusinessOwner::Organization(rival)
    );
    assert_eq!(
        owner(&fixture, player_shop),
        BusinessOwner::Organization(player)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(merchant))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| account.balance().cents())
            .sum::<i64>(),
        purchase_price
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * 1440));
    assert!(apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state).is_empty());
    assert_eq!(
        owner(&fixture, player_shop),
        BusinessOwner::Organization(player)
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn staffed_seller_retains_earning_base_until_a_funded_replacement_exists() {
    let mut fixture = make_independent_fixture();
    let player = fixture.organization;
    designate_player_organization(&mut fixture.state, player).unwrap();
    let seller = organization(&mut fixture, "Rival", OrganizationKind::Criminal);
    executive(&mut fixture, seller);
    establish_existing_independent_economy(&mut fixture);
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(seller),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let offer = Money::from_cents(hospitality_price(&fixture).cents() * 100);
    fund_accounted_from_street(&mut fixture, offer.cents());
    // An existing liability must not swallow the consideration that justified the sale.
    let indebted_account = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(seller),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Existing seller debt".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: indebted_account,
                    amount: offer.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: offer,
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    // The acquisition has not served its investment period, so even a feasible replacement
    // cannot cause an immediate flip.
    let replacement = another_property(&mut fixture, BusinessOwner::Independent, true);
    assert!(matches!(
        validate_offer_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
            offer
        ),
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(91 * 1440));
    let held = validate_offer_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
        offer,
    )
    .unwrap();
    let mut unavailable = fixture.state.clone();
    validate_transfer_business_ownership(
        &unavailable,
        replacement,
        BusinessOwner::Organization(player),
    )
    .unwrap()
    .commit(&mut unavailable)
    .unwrap();
    let before = bincode::serialize(&unavailable).unwrap();
    assert_eq!(
        held.commit(&fixture.registry, &mut unavailable)
            .unwrap_err(),
        BusinessAcquisitionError::OfferDeclined
    );
    assert_eq!(bincode::serialize(&unavailable).unwrap(), before);
    assert!(matches!(
        validate_offer_business(
            &fixture.registry,
            &unavailable,
            acquisition_draft(&fixture),
            offer
        ),
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    validate_offer_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
        offer,
    )
    .unwrap()
    .commit(&fixture.registry, &mut fixture.state)
    .unwrap();
    assert_eq!(
        owner(&fixture, fixture.business),
        BusinessOwner::Organization(player)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(indebted_account)
            .unwrap()
            .balance(),
        offer.checked_neg().unwrap()
    );
    let spendable_proceeds = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(seller))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.available_payment_balance().cents())
        .sum::<i64>();
    assert_eq!(spendable_proceeds, offer.cents());
    fixture.state = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    // The seller has received the consideration and can reinvest without any existing holdings.
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![replacement]
    );
    assert_eq!(
        owner(&fixture, replacement),
        BusinessOwner::Organization(seller)
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}
