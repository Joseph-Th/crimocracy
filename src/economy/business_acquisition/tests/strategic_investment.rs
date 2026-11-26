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
fn proprietor_requires_livelihood_compensation_when_other_shop_closes() {
    use crate::economy::business_economy_system::{
        validate_disrupt_business_economy, validate_resume_business_economy,
        validate_suspend_business_economy,
    };

    let mut fixture = make_independent_fixture();
    let proprietor = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Private proprietor".into(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    establish_existing_independent_economy(&mut fixture);
    let property = fixture.business;
    validate_transfer_business_ownership(
        &fixture.state,
        property,
        BusinessOwner::Character(proprietor),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let minimum = |registry: &Registry, state: &AppState| {
        super::super::personal_seller_minimum(registry, state, proprietor, property).unwrap()
    };
    let sole_livelihood = minimum(&fixture.registry, &fixture.state);
    let other = another_property(&mut fixture, BusinessOwner::Character(proprietor), true);
    let diversified = minimum(&fixture.registry, &fixture.state);
    assert!(diversified < sole_livelihood);
    let mut damaged = fixture.state.clone();
    let public_quote = appraise_business(&fixture.registry, &damaged, property).unwrap();
    validate_disrupt_business_economy(&fixture.registry, &damaged, other)
        .unwrap()
        .commit(&mut damaged)
        .unwrap();
    assert!(
        business_economy_system::forecast_live_business_net(&fixture.registry, &damaged, other)
            .unwrap()
            < Money::ZERO
    );
    assert_eq!(
        minimum(&fixture.registry, &damaged),
        sole_livelihood,
        "an operating but loss-making alternative does not replace the owner's livelihood"
    );
    assert_eq!(
        appraise_business(&fixture.registry, &damaged, property).unwrap(),
        public_quote,
        "the proprietor's private circumstances must not change the public appraisal"
    );
    validate_invariants(&damaged);
    let price = Money::from_cents(i64::try_from(diversified).unwrap());
    fund_accounted_from_street(&mut fixture, price.cents());
    let offer = validate_offer_business(
        &fixture.registry,
        &fixture.state,
        acquisition_draft(&fixture),
        price,
    )
    .unwrap();

    validate_suspend_business_economy(&fixture.state, other)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        minimum(&fixture.registry, &fixture.state),
        sole_livelihood,
        "ownership of an inactive storefront does not replace a productive livelihood"
    );
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        offer.commit(&fixture.registry, &mut fixture.state),
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(minimum(&fixture.registry, &restored), sole_livelihood);
    for state in [&mut fixture.state, &mut restored] {
        assert!(matches!(
            validate_offer_business(
                &fixture.registry,
                state,
                BusinessAcquisitionDraft {
                    organization: fixture.organization,
                    business: property,
                    funding_accounts: BTreeSet::from([fixture.accounted]),
                },
                price,
            ),
            Err(BusinessAcquisitionError::OfferDeclined)
        ));
        validate_resume_business_economy(&fixture.registry, state, other)
            .unwrap()
            .commit(state)
            .unwrap();
        assert_eq!(minimum(&fixture.registry, state), diversified);
        validate_offer_business(
            &fixture.registry,
            state,
            BusinessAcquisitionDraft {
                organization: fixture.organization,
                business: property,
                funding_accounts: BTreeSet::from([fixture.accounted]),
            },
            price,
        )
        .unwrap()
        .commit(&fixture.registry, state)
        .unwrap();
        assert_eq!(
            state.world().get_business(property).unwrap().owner(),
            BusinessOwner::Organization(fixture.organization)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn ambiguous_rival_executive_cannot_commission_acquisition() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    let player = organization(&mut fixture, "Player", OrganizationKind::Criminal);
    designate_player_organization(&mut fixture.state, player).unwrap();
    executive(&mut fixture, rival);
    executive(&mut fixture, rival);
    establish_existing_independent_economy(&mut fixture);
    let price = hospitality_price(&fixture).cents();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 14;
    fund_accounted_from_street(&mut fixture, price + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * 1440));
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state).is_empty());
    assert_eq!(
        owner(&fixture, fixture.business),
        BusinessOwner::Independent
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
}

#[test]
fn staffed_seller_requires_a_sole_available_executive_to_accept_offer() {
    use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft,
    };
    use crate::world::world_system::validate_set_character_supervisor;

    let mut fixture = make_independent_fixture();
    designate_player_organization(&mut fixture.state, fixture.organization).unwrap();
    let seller = organization(&mut fixture, "Staffed seller", OrganizationKind::Criminal);
    executive(&mut fixture, seller);
    let leader = fixture
        .state
        .world()
        .characters_in_organization(seller)
        .next()
        .unwrap()
        .id();
    let target = another_property(&mut fixture, BusinessOwner::Organization(seller), true);
    let offer = Money::from_cents(hospitality_price(&fixture).cents() * 100);
    fund_accounted_from_street(&mut fixture, offer.cents());
    let draft = BusinessAcquisitionDraft {
        organization: fixture.organization,
        business: target,
        funding_accounts: BTreeSet::from([fixture.accounted]),
    };
    let held =
        validate_offer_business(&fixture.registry, &fixture.state, draft.clone(), offer).unwrap();
    executive(&mut fixture, seller);
    let competing_leader = fixture
        .state
        .world()
        .characters_in_organization(seller)
        .find(|person| person.id() != leader)
        .unwrap()
        .id();
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        held.commit(&fixture.registry, &mut fixture.state)
            .unwrap_err(),
        BusinessAcquisitionError::OfferDeclined
    );
    assert_eq!(
        validate_offer_business(&fixture.registry, &fixture.state, draft.clone(), offer).err(),
        Some(BusinessAcquisitionError::OfferDeclined)
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);

    // Consolidating real command restores consent. A subsequent arrest of that
    // executive must invalidate both a held offer and newly submitted offers.
    validate_set_character_supervisor(&fixture.state, competing_leader, Some(leader))
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let held =
        validate_offer_business(&fixture.registry, &fixture.state, draft.clone(), offer).unwrap();
    let police = organization(&mut fixture, "Police", OrganizationKind::LawEnforcement);
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: police,
            title: "Seller executive custody".into(),
            subjects: BTreeSet::from([EntityRef::Character(leader)]),
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let mut evidence = BTreeSet::new();
    for (kind, strength) in [
        (EvidenceKind::FinancialRecord, EvidenceStrength::Strong),
        (EvidenceKind::Document, EvidenceStrength::Corroborating),
    ] {
        evidence.insert(
            validate_add_evidence(
                &fixture.state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject: EntityRef::Character(leader),
                    origin: None,
                    kind,
                    strength,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: fixture.state.now(),
                },
            )
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap(),
        );
    }
    let arrest = validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character: leader,
            investigation,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        held.commit(&fixture.registry, &mut fixture.state)
            .unwrap_err(),
        BusinessAcquisitionError::OfferDeclined
    );
    assert_eq!(
        validate_offer_business(&fixture.registry, &fixture.state, draft.clone(), offer).err(),
        Some(BusinessAcquisitionError::OfferDeclined)
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);

    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_release_arrest(state, arrest)
            .unwrap()
            .commit(state)
            .unwrap();
        let receipt = validate_offer_business(&fixture.registry, state, draft.clone(), offer)
            .unwrap()
            .commit(&fixture.registry, state)
            .unwrap();
        assert_eq!(receipt.business, target);
        assert_eq!(receipt.price, offer);
        assert_eq!(
            state.world().get_business(target).unwrap().owner(),
            BusinessOwner::Organization(fixture.organization)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn equally_funded_rivals_alternate_first_access_to_scarce_listings_across_weeks() {
    let mut fixture = make_independent_fixture();
    let first = fixture.organization;
    let second = organization(&mut fixture, "Second rival", OrganizationKind::Criminal);
    let player = organization(&mut fixture, "Player", OrganizationKind::Criminal);
    designate_player_organization(&mut fixture.state, player).unwrap();
    executive(&mut fixture, first);
    executive(&mut fixture, second);
    establish_existing_independent_economy(&mut fixture);

    let price = hospitality_price(&fixture).cents();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    let cash = price + reserve;
    fund_accounted_from_street(&mut fixture, cash);
    let second_accounted = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(second),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let second_street = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(second),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Second rival investment reserves".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: second_street,
                    amount: Money::from_cents(-cash),
                },
                LedgerPosting {
                    account: second_accounted,
                    amount: Money::from_cents(cash),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();

    let opening = fixture.state.clone();
    for (week, expected_buyer) in [(1, first), (2, second), (3, first)] {
        let mut state = opening.clone();
        state.advance_clock(SimDuration::from_minutes(week * 7 * 1440));
        assert_eq!(
            apply_due_autonomous_acquisitions(&fixture.registry, &mut state),
            vec![fixture.business]
        );
        assert_eq!(
            state
                .world()
                .get_business(fixture.business)
                .unwrap()
                .owner(),
            BusinessOwner::Organization(expected_buyer)
        );
        let restored = restore_save(
            &fixture.registry,
            build_save(&fixture.registry, &state).unwrap(),
        )
        .expect("rotated investment must preserve valid save state");
        assert_eq!(
            bincode::serialize(&state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
    }
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
    // The ordinary public reference offer must not flip a recent acquisition.
    // This unusually generous offer can buy out the investment period, but it
    // still has to satisfy the seller's private wage-base and replacement rules.
    let replacement = another_property(&mut fixture, BusinessOwner::Independent, true);
    let ordinary_offer = Money::from_cents(
        i64::try_from(
            negotiated_reference_price(&fixture.registry, &fixture.state, fixture.business)
                .unwrap(),
        )
        .unwrap(),
    );
    assert!(matches!(
        validate_offer_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
            ordinary_offer
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

#[test]
fn mature_seller_can_accept_full_years_payroll_without_a_replacement() {
    let mut fixture = make_independent_fixture();
    let player = fixture.organization;
    designate_player_organization(&mut fixture.state, player).unwrap();
    let seller = organization(&mut fixture, "Mature seller", OrganizationKind::Criminal);
    executive(&mut fixture, seller);
    let leader = fixture
        .state
        .world()
        .characters_in_organization(seller)
        .next()
        .unwrap()
        .id();
    for index in 0..4 {
        insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: format!("Mature seller employee {index}"),
                organization: Some(seller),
                supervisor: Some(leader),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
    establish_existing_independent_economy(&mut fixture);
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(seller),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(91 * 1440));

    // There is no independent replacement anywhere in this fixture.
    let daily_payroll = i128::from(fixture.registry.upkeep().per_member_daily().cents()) * 5;
    let runway = daily_payroll * (super::super::seller_strategy::SALE_RUNWAY_DAYS + 7);
    let reference =
        negotiated_reference_price(&fixture.registry, &fixture.state, fixture.business).unwrap();
    assert!(
        reference < runway,
        "ordinary appraisal must not fund this seller's wage gap"
    );
    let price = Money::from_cents(i64::try_from(runway).unwrap());
    fund_accounted_from_street(&mut fixture, price.cents());

    assert!(matches!(
        validate_offer_business(
            &fixture.registry,
            &fixture.state,
            acquisition_draft(&fixture),
            Money::from_cents(i64::try_from(reference).unwrap()),
        ),
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    let before = bincode::serialize(&fixture.state).unwrap();
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_offer_business(
            &fixture.registry,
            state,
            BusinessAcquisitionDraft {
                organization: player,
                business: fixture.business,
                funding_accounts: BTreeSet::from([fixture.accounted]),
            },
            price,
        )
        .unwrap()
        .commit(&fixture.registry, state)
        .unwrap();
        assert_eq!(
            state
                .world()
                .get_business(fixture.business)
                .unwrap()
                .owner(),
            BusinessOwner::Organization(player)
        );
        let seller_liquidity: i128 = state
            .finance()
            .accounts_for(FinancialOwner::Organization(seller))
            .filter(|account| account.kind().is_liquid())
            .map(|account| i128::from(account.balance().cents()))
            .sum();
        assert!(seller_liquidity >= runway);
        validate_invariants(state);
    }
    assert_ne!(bincode::serialize(&fixture.state).unwrap(), before);
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn recently_acquired_property_requires_exceptional_buyout_not_routine_resale() {
    let mut fixture = make_independent_fixture();
    let player = fixture.organization;
    designate_player_organization(&mut fixture.state, player).unwrap();
    let seller = organization(&mut fixture, "Protected seller", OrganizationKind::Criminal);
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
    let other = another_property(&mut fixture, BusinessOwner::Organization(seller), false);
    let retained_income =
        super::super::seller_strategy::daily_net(&fixture.registry, &fixture.state, other).unwrap();
    assert!(
        retained_income >= i128::from(fixture.registry.upkeep().per_member_daily().cents()),
        "the remaining active holding must cover the sole seller executive's wages"
    );
    let appraisal = appraise_business(&fixture.registry, &fixture.state, fixture.business).unwrap();
    let daily_net = super::super::seller_strategy::daily_net(
        &fixture.registry,
        &fixture.state,
        fixture.business,
    )
    .unwrap();
    let buyout_floor = i128::from(appraisal.standard_price.cents()) + 180 * daily_net;
    assert!(buyout_floor > appraisal.reference_offer_cents);
    let buyout = Money::from_cents(i64::try_from(buyout_floor).unwrap());
    let below_buyout = Money::from_cents(buyout.cents() - 1);
    fund_accounted_from_street(&mut fixture, buyout.cents());

    let before = bincode::serialize(&fixture.state).unwrap();
    for ordinary in [
        Money::from_cents(i64::try_from(appraisal.reference_offer_cents).unwrap()),
        below_buyout,
    ] {
        assert!(matches!(
            validate_offer_business(
                &fixture.registry,
                &fixture.state,
                acquisition_draft(&fixture),
                ordinary
            ),
            Err(BusinessAcquisitionError::OfferDeclined)
        ));
        assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    }
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_offer_business(
            &fixture.registry,
            state,
            BusinessAcquisitionDraft {
                organization: player,
                business: fixture.business,
                funding_accounts: BTreeSet::from([fixture.accounted]),
            },
            buyout,
        )
        .unwrap()
        .commit(&fixture.registry, state)
        .unwrap();
        assert_eq!(
            state
                .world()
                .get_business(fixture.business)
                .unwrap()
                .owner(),
            BusinessOwner::Organization(player)
        );
        assert_eq!(
            state.world().get_business(other).unwrap().owner(),
            BusinessOwner::Organization(seller)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}
