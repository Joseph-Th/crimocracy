//! Rival purchase planning must account for the real economics of reopening a closed shop.

use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::DAY_MINUTES;
use crate::economy::BusinessOperatingStatus;
use crate::economy::business_economy_system::validate_suspend_business_economy;
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

fn established_shop(
    fixture: &mut AcquisitionFixture,
    kind: BusinessKind,
    owner: BusinessOwner,
    neighborhood: crate::core::id::NeighborhoodId,
) -> BusinessId {
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: format!("{kind:?} shop"),
            kind,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood,
            owner,
        },
    )
    .unwrap();
    let original = fixture.business;
    fixture.business = business;
    establish_existing_independent_economy(fixture);
    fixture.business = original;
    business
}

#[test]
fn rival_avoids_reopening_positive_margin_shop_that_destroys_its_other_earnings() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    let player = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Player organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut fixture.state, player).unwrap();
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Marginal retail ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(20),
                    commercial_activity: rating(21),
                    illicit_demand: rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(100),
                },
            },
        },
    )
    .unwrap();

    let dormant = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    let pharmacy = established_shop(
        &mut fixture,
        BusinessKind::Pharmacy,
        BusinessOwner::Organization(rival),
        neighborhood,
    );
    validate_suspend_business_economy(&fixture.state, dormant)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let appraisal = appraise_business(&fixture.registry, &fixture.state, dormant).unwrap();
    assert!(appraisal.expected_cycle_net > Money::ZERO);
    let lost_daily_net = business_economy_system::acquisition_owner_daily_displacement(
        &fixture.registry,
        &fixture.state,
        dormant,
        rival,
    )
    .unwrap();
    let incoming_daily_net = i128::from(appraisal.expected_cycle_net.cents())
        * i128::from(DAY_MINUTES)
        / i128::from(appraisal.cycle_minutes);
    assert!(lost_daily_net > incoming_daily_net);

    let price = fixture
        .registry
        .get_business(BusinessKind::Retail)
        .economics()
        .acquisition_cost();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price.cents() * 2 + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert!(apply_due_autonomous_acquisitions(&fixture.registry, state).is_empty());
        assert_eq!(
            state.world().get_business(dormant).unwrap().owner(),
            BusinessOwner::Independent
        );
        assert_eq!(
            state.world().get_business(pharmacy).unwrap().owner(),
            BusinessOwner::Organization(rival)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );

    // This is an autonomous economic choice, not a new player-acquisition restriction.
    let purchase = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: rival,
            business: dormant,
            funding_accounts: BTreeSet::from([fixture.accounted]),
        },
    )
    .unwrap();
    purchase
        .commit(&fixture.registry, &mut fixture.state)
        .unwrap();
    validate_invariants(&fixture.state);
}

#[test]
fn rival_reopens_viable_dormant_listing_without_harming_its_portfolio() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Available executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap()
        .neighborhood();
    let dormant = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    validate_suspend_business_economy(&fixture.state, dormant)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        business_economy_system::acquisition_owner_daily_displacement(
            &fixture.registry,
            &fixture.state,
            dormant,
            rival,
        )
        .unwrap(),
        0,
    );
    let price = fixture
        .registry
        .get_business(BusinessKind::Retail)
        .economics()
        .acquisition_cost();
    assert!(
        appraise_business(&fixture.registry, &fixture.state, dormant)
            .unwrap()
            .expected_cycle_net
            > Money::ZERO
    );
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price.cents() + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![dormant],
    );
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(dormant)
            .unwrap()
            .status(),
        BusinessOperatingStatus::Active,
    );
    validate_invariants(&fixture.state);
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}
