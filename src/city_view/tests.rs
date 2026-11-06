use super::*;
use crate::build_registry;
use crate::core::persistence::{build_save, restore_save};
use crate::core::state::AppState;
use crate::world::world_system::{insert_business, insert_neighborhood};
use crate::world::{
    ALL_BUSINESS_KINDS, BusinessDraft, BusinessOwner, NeighborhoodDraft,
    NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, Rating,
};
use std::collections::BTreeSet;

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("valid fixture rating")
}

fn add_neighborhood(state: &mut AppState, name: &str) -> NeighborhoodId {
    insert_neighborhood(
        state,
        NeighborhoodDraft {
            name: name.to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(75),
                    commercial_activity: rating(65),
                    illicit_demand: rating(40),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(55),
                },
            },
        },
    )
    .expect("valid neighborhood")
}

fn add_business(
    state: &mut AppState,
    neighborhood: NeighborhoodId,
    kind: BusinessKind,
) -> BusinessId {
    insert_business(
        &build_registry(),
        state,
        BusinessDraft {
            name: format!("Business {kind:?}"),
            kind,
            functions: BTreeSet::new(),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("valid business")
}

#[test]
fn generated_city_is_deterministic_and_does_not_mutate_simulation_state() {
    let registry = build_registry();
    let mut state = AppState::new(99);
    let first = add_neighborhood(&mut state, "South Ward");
    let second = add_neighborhood(&mut state, "Waterfront");
    let market = add_business(&mut state, first, BusinessKind::Retail);
    let dock = add_business(&mut state, second, BusinessKind::Stevedoring);

    let before = bincode::serialize(&state).expect("serialize campaign");
    let scene = CityScene::generate(state.world(), 457).expect("generate scene");
    let again = CityScene::generate(state.world(), 457).expect("regenerate scene");
    assert_eq!(scene, again);
    assert_eq!(before, bincode::serialize(&state).unwrap());

    let saved = build_save(&registry, &state).expect("valid save");
    let restored = restore_save(&registry, saved).expect("valid restore");
    let restored_scene = CityScene::generate(restored.world(), 457).unwrap();
    assert_eq!(scene, restored_scene);
    assert_eq!(scene.business_from_pick_id(market.raw()), Some(market));
    assert_eq!(scene.business_from_pick_id(dock.raw()), Some(dock));
    assert_eq!(scene.business_from_pick_id(0), None);
}

#[test]
fn adding_a_business_does_not_move_preexisting_buildings_or_other_districts() {
    let mut state = AppState::new(7);
    let first = add_neighborhood(&mut state, "Central");
    let second = add_neighborhood(&mut state, "East");
    let old = add_business(&mut state, first, BusinessKind::Warehouse);
    let scene = CityScene::generate(state.world(), 0xCAFE).unwrap();
    let original_pin = scene.business_pin(old).unwrap();
    let unaffected = scene.districts[1].clone();

    let new = add_business(&mut state, first, BusinessKind::Nightclub);
    let expanded = CityScene::generate(state.world(), 0xCAFE).unwrap();
    assert_eq!(expanded.business_pin(old), Some(original_pin));
    assert_ne!(expanded.business_pin(new), Some(original_pin));
    assert_eq!(expanded.districts[1], unaffected);
    assert_eq!(expanded.business_pin(new).unwrap().neighborhood, first);
    assert_eq!(unaffected.neighborhood, second);
}

#[test]
fn batches_contain_every_lot_once_with_unique_business_picks() {
    let mut state = AppState::new(1);
    let neighborhood = add_neighborhood(&mut state, "Industrial");
    let ids: Vec<_> = ALL_BUSINESS_KINDS
        .iter()
        .map(|&kind| add_business(&mut state, neighborhood, kind))
        .collect();
    let scene = CityScene::generate(state.world(), 18).unwrap();
    let district = &scene.districts[0];
    assert_eq!(district.buildings.len(), BuildingMesh::ALL.len());
    assert_eq!(district.streets.len(), (LOTS_PER_SIDE + 1) * 2);
    assert_eq!(
        district
            .buildings
            .iter()
            .map(|b| b.instances.len())
            .sum::<usize>(),
        LOT_CAPACITY
    );
    let picks: Vec<_> = district
        .buildings
        .iter()
        .flat_map(|b| b.instances.iter())
        .filter_map(|instance| (instance.pick_id != 0).then_some(instance.pick_id))
        .collect();
    assert_eq!(picks.len(), ids.len());
    assert_eq!(
        picks.iter().copied().collect::<BTreeSet<_>>().len(),
        ids.len()
    );
    for id in ids {
        assert!(scene.business_pin(id).is_some());
        assert_eq!(scene.business_from_pick_id(id.raw()), Some(id));
    }
    assert_eq!(scene.business_from_pick_id(u32::MAX), None);
}

#[test]
fn spatial_culling_uses_fixed_district_bounds_without_loading_unrelated_chunks() {
    let mut state = AppState::new(3);
    for index in 0..9 {
        add_neighborhood(&mut state, &format!("District {index}"));
    }
    let scene = CityScene::generate(state.world(), 11).unwrap();
    let first = &scene.districts[0];
    let second_row = &scene.districts[8];
    assert_eq!(first.bounds.min_x, -2.0);
    assert_eq!(second_row.bounds.min_z, DISTRICT_PITCH - 2.0);
    let viewport = Bounds2 {
        min_x: 0.0,
        min_z: 0.0,
        max_x: 100.0,
        max_z: 100.0,
    };
    let visible: Vec<_> = scene
        .visible_districts(viewport)
        .map(|district| district.neighborhood)
        .collect();
    assert_eq!(visible, vec![first.neighborhood]);
    assert!(!first.bounds.intersects(second_row.bounds));
}

#[test]
fn visual_seed_changes_art_not_gameplay_or_entity_identity() {
    let mut state = AppState::new(42);
    let district = add_neighborhood(&mut state, "Market");
    let business = add_business(&mut state, district, BusinessKind::Retail);
    let one = CityScene::generate(state.world(), 123).unwrap();
    let two = CityScene::generate(state.world(), 456).unwrap();
    assert_ne!(one.districts[0].buildings, two.districts[0].buildings);
    assert_eq!(one.business_from_pick_id(business.raw()), Some(business));
    assert_eq!(two.business_from_pick_id(business.raw()), Some(business));
}

#[test]
fn exceeding_district_capacity_returns_typed_error_without_dropping_a_business() {
    let registry = build_registry();
    let mut state = AppState::new(8);
    let neighborhood = add_neighborhood(&mut state, "Crowded");
    for index in 0..=LOT_CAPACITY {
        insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: format!("Shop {index}"),
                kind: BusinessKind::Retail,
                functions: BTreeSet::new(),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("valid business");
    }
    assert_eq!(
        CityScene::generate(state.world(), 0),
        Err(CityViewError::DistrictCapacity {
            neighborhood,
            businesses: LOT_CAPACITY + 1,
            capacity: LOT_CAPACITY,
        })
    );
}

#[test]
fn refreshing_one_neighborhood_matches_full_regeneration_and_preserves_other_chunks() {
    let mut state = AppState::new(19);
    let first = add_neighborhood(&mut state, "Harbor");
    let second = add_neighborhood(&mut state, "Uptown");
    let old = add_business(&mut state, first, BusinessKind::Warehouse);
    let mut scene = CityScene::generate(state.world(), 615).unwrap();
    let untouched = scene.districts[1].clone();
    let old_pin = scene.business_pin(old);

    let new_business = add_business(&mut state, first, BusinessKind::Nightclub);
    scene.refresh_neighborhood(state.world(), first).unwrap();
    assert_eq!(scene, CityScene::generate(state.world(), 615).unwrap());
    assert_eq!(scene.districts[1], untouched);
    assert_eq!(scene.business_pin(old), old_pin);
    assert!(scene.business_pin(new_business).is_some());

    let third = add_neighborhood(&mut state, "Hillside");
    scene.refresh_neighborhood(state.world(), third).unwrap();
    assert_eq!(scene, CityScene::generate(state.world(), 615).unwrap());
    assert_eq!(scene.districts[1].neighborhood, second);
}

#[test]
fn failed_neighborhood_refresh_keeps_the_previous_scene_and_picking_index() {
    let registry = build_registry();
    let mut state = AppState::new(20);
    let neighborhood = add_neighborhood(&mut state, "Market");
    let first = add_business(&mut state, neighborhood, BusinessKind::Retail);
    let mut scene = CityScene::generate(state.world(), 5).unwrap();
    let before = scene.clone();
    assert_eq!(
        scene.refresh_neighborhood(state.world(), NeighborhoodId::from_raw(999)),
        Err(CityViewError::MissingNeighborhood(
            NeighborhoodId::from_raw(999)
        ))
    );
    assert_eq!(scene, before);
    for index in 1..=LOT_CAPACITY {
        insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: format!("Overflow {index}"),
                kind: BusinessKind::Retail,
                functions: BTreeSet::new(),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .unwrap();
    }
    assert_eq!(
        scene.refresh_neighborhood(state.world(), neighborhood),
        Err(CityViewError::DistrictCapacity {
            neighborhood,
            businesses: LOT_CAPACITY + 1,
            capacity: LOT_CAPACITY,
        })
    );
    assert_eq!(scene, before);
    assert_eq!(scene.business_from_pick_id(first.raw()), Some(first));
}
