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
fn explicit_city_grid_width_preserves_determinism_positions_and_incremental_updates() {
    let registry = build_registry();
    let mut state = AppState::new(13);
    for index in 0..8 {
        add_neighborhood(&mut state, &format!("Ward {index}"));
    }
    let business = add_business(
        &mut state,
        NeighborhoodId::from_raw(6),
        BusinessKind::Warehouse,
    );
    assert_eq!(
        CityScene::generate_with_columns(state.world(), 90, 0),
        Err(CityViewError::InvalidDistrictColumns(0)),
    );
    assert_eq!(
        CityScene::generate_with_columns(state.world(), 90, 33),
        Err(CityViewError::InvalidDistrictColumns(33)),
    );
    let original = CityScene::generate_with_columns(state.world(), 90, 4).unwrap();
    assert_eq!(
        original.districts[3].bounds.min_x,
        DISTRICT_PITCH * 3.0 - 5.0
    );
    assert_eq!(original.districts[4].bounds.min_x, -5.0);
    assert_eq!(original.districts[4].bounds.min_z, DISTRICT_PITCH - 5.0);
    assert_eq!(original.districts[7].bounds.min_z, DISTRICT_PITCH - 5.0);
    assert_ne!(original, CityScene::generate(state.world(), 90).unwrap());

    let saved = build_save(&registry, &state).unwrap();
    let restored = restore_save(&registry, saved).unwrap();
    assert_eq!(
        CityScene::generate_with_columns(restored.world(), 90, 4).unwrap(),
        original,
    );

    let ninth = add_neighborhood(&mut state, "New Ward");
    let mut updated = original.clone();
    updated.refresh_neighborhood(state.world(), ninth).unwrap();
    assert_eq!(
        updated,
        CityScene::generate_with_columns(state.world(), 90, 4).unwrap(),
    );
    assert_eq!(updated.districts[8].bounds.min_x, -5.0);
    assert_eq!(
        updated.districts[8].bounds.min_z,
        DISTRICT_PITCH * 2.0 - 5.0
    );
    assert_eq!(
        updated.business_pin(business),
        original.business_pin(business)
    );
    assert_eq!(&updated.districts[..8], &original.districts[..]);
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
    assert!(district.streets.len() >= (LOTS_PER_SIDE / BLOCK_SIDE_IN_LOTS + 1) * 2);
    assert!(district.streets.len() <= (LOTS_PER_SIDE / BLOCK_SIDE_IN_LOTS + 1) * 2 + 16);
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
    assert_eq!(first.bounds.min_x, -5.0);
    assert_eq!(second_row.bounds.min_z, DISTRICT_PITCH - 5.0);
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
    // Road strips cross the district boundary by design; their culling
    // bounds overlap even though the developed building lots do not.
    assert!(first.bounds.intersects(second_row.bounds));
}

#[test]
fn generated_building_footprints_do_not_intersect_procedural_roads() {
    let mut state = AppState::new(77);
    add_neighborhood(&mut state, "Downtown");
    let scene = CityScene::generate(state.world(), 0xDEAD_BEEF).unwrap();
    for district in &scene.districts {
        for building in district.buildings.iter().flat_map(|batch| &batch.instances) {
            let [x, _, z] = building.transform.translation;
            let [width, _, depth] = building.transform.scale;
            assert!(width > 0.0 && depth > 0.0);
            for road in &district.streets {
                let [road_x, _, road_z] = road.translation;
                let [road_width, _, road_depth] = road.scale;
                let intersects_x = (x - road_x).abs() * 2.0 < width + road_width;
                let intersects_z = (z - road_z).abs() * 2.0 < depth + road_depth;
                assert!(
                    !(intersects_x && intersects_z),
                    "building at ({x},{z}) intersects street at ({road_x},{road_z})"
                );
            }
        }
    }
}

#[test]
fn neighboring_district_arterials_join_without_gaps() {
    let mut state = AppState::new(42);
    for index in 0..9 {
        add_neighborhood(&mut state, &format!("Ward {index}"));
    }
    let scene = CityScene::generate(state.world(), 123).unwrap();
    let origin = &scene.districts[0];
    let east = &scene.districts[1];
    let south = &scene.districts[8];

    // The main east-west road at z=0 continues through the district gap.
    let east_west = |chunk: &DistrictChunk| {
        *chunk
            .streets
            .iter()
            .find(|street| street.scale[0] > street.scale[2] && street.translation[2] == 0.0)
            .expect("horizontal arterial must span the district")
    };
    let road = east_west(origin);
    let extension = east_west(east);
    let first_end = road.translation[0] + road.scale[0] * 0.5;
    let next_start = extension.translation[0] - extension.scale[0] * 0.5;
    assert!(first_end >= next_start);
    assert_eq!(road.scale[2], ARTERIAL_WIDTH);
    assert_eq!(extension.scale[2], ARTERIAL_WIDTH);

    // The north-south road at x=0 likewise crosses the next row's margin.
    let north_south = |chunk: &DistrictChunk| {
        *chunk
            .streets
            .iter()
            .find(|street| street.scale[2] > street.scale[0] && street.translation[0] == 0.0)
            .expect("vertical arterial must span the district")
    };
    let road = north_south(origin);
    let extension = north_south(south);
    let first_end = road.translation[2] + road.scale[2] * 0.5;
    let next_start = extension.translation[2] - extension.scale[2] * 0.5;
    assert!(first_end >= next_start);
}

#[test]
fn generated_streets_are_fully_covered_by_their_district_culling_bounds() {
    let mut state = AppState::new(11);
    for index in 0..16 {
        add_neighborhood(&mut state, &format!("Block {index}"));
    }
    for seed in [0, 1, 143, u64::MAX] {
        let scene = CityScene::generate(state.world(), seed).unwrap();
        for chunk in &scene.districts {
            for street in &chunk.streets {
                let [x, _, z] = street.translation;
                let [width, _, depth] = street.scale;
                assert!(x - width * 0.5 >= chunk.bounds.min_x);
                assert!(x + width * 0.5 <= chunk.bounds.max_x);
                assert!(z - depth * 0.5 >= chunk.bounds.min_z);
                assert!(z + depth * 0.5 <= chunk.bounds.max_z);
            }
        }
    }
}

#[test]
fn sector_instance_ranges_partition_every_mesh_buffer_without_duplication() {
    let mut state = AppState::new(80);
    let district = add_neighborhood(&mut state, "Harbor");
    let disclosed: BTreeSet<_> = ALL_BUSINESS_KINDS
        .iter()
        .map(|&kind| add_business(&mut state, district, kind))
        .collect();
    let scene = CityScene::generate(state.world(), 911).unwrap();
    let chunk = &scene.districts[0];
    assert_eq!(chunk.sectors.len(), SECTORS_PER_SIDE * SECTORS_PER_SIDE);
    let gpu = GpuCityChunk::encode(chunk, &disclosed);
    let mut covered = 0;
    for (mesh_index, batch) in chunk.buildings.iter().enumerate() {
        let mut expected_first = 0;
        for sector in &chunk.sectors {
            let range = sector.building_ranges[mesh_index];
            assert_eq!(range.first, expected_first);
            expected_first += range.count;
            assert!((range.first + range.count) as usize <= batch.instances.len());
            for instance in &batch.instances[range.first as usize..expected_first as usize] {
                assert!(sector.bounds.min_x <= instance.transform.translation[0]);
                assert!(instance.transform.translation[0] <= sector.bounds.max_x);
                assert!(sector.bounds.min_z <= instance.transform.translation[2]);
                assert!(instance.transform.translation[2] <= sector.bounds.max_z);
            }
        }
        assert_eq!(expected_first as usize, batch.instances.len());
        covered += expected_first;
        if let Some(upload) = gpu
            .buildings
            .iter()
            .find(|upload| upload.mesh == batch.mesh)
        {
            assert_eq!(upload.instance_count, expected_first);
            assert_eq!(
                upload.bytes.len(),
                batch.instances.len() * GPU_INSTANCE_STRIDE
            );
        } else {
            assert_eq!(expected_first, 0);
        }
    }
    assert_eq!(covered as usize, LOT_CAPACITY);
    let close_view = Bounds2 {
        min_x: 40.0,
        min_z: 40.0,
        max_x: 60.0,
        max_z: 60.0,
    };
    let visible: Vec<_> = chunk.visible_sectors(close_view).collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(
        visible[0]
            .building_ranges
            .iter()
            .map(|range| range.count)
            .sum::<u32>(),
        64
    );
}

#[test]
fn viewport_query_matches_exhaustive_culling_across_rows_and_invalid_boundaries() {
    let mut state = AppState::new(27);
    for index in 0..84 {
        add_neighborhood(&mut state, &format!("District {index}"));
    }
    let scene = CityScene::generate(state.world(), 29).unwrap();
    let sample_viewports = [
        Bounds2 {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 50.0,
            max_z: 50.0,
        },
        Bounds2 {
            min_x: -100.0,
            min_z: -100.0,
            max_x: 100.0,
            max_z: 100.0,
        },
        Bounds2 {
            min_x: 700.0,
            min_z: 700.0,
            max_x: 810.0,
            max_z: 810.0,
        },
        Bounds2 {
            min_x: 0.0,
            min_z: 7000.0,
            max_x: 6000.0,
            max_z: 9000.0,
        },
        Bounds2 {
            min_x: -100.0,
            min_z: 90000.0,
            max_x: 10000.0,
            max_z: 100000.0,
        },
        Bounds2 {
            min_x: 0.0,
            min_z: f32::NAN,
            max_x: 800.0,
            max_z: 800.0,
        },
        Bounds2 {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 800.0,
            max_z: f32::NAN,
        },
        Bounds2 {
            min_x: 80.0,
            min_z: 80.0,
            max_x: 20.0,
            max_z: 20.0,
        },
    ];
    for viewport in sample_viewports {
        let expected: Vec<_> = scene
            .districts
            .iter()
            .filter(|chunk| chunk.bounds.intersects(viewport))
            .map(|chunk| chunk.neighborhood)
            .collect();
        let actual: Vec<_> = scene
            .visible_districts(viewport)
            .map(|chunk| chunk.neighborhood)
            .collect();
        assert_eq!(actual, expected, "viewport {viewport:?}");
        if !viewport.is_valid() {
            assert!(actual.is_empty());
        }
    }
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
    for index in 0..LOT_CAPACITY + 4 {
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
            businesses: LOT_CAPACITY + 4,
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
