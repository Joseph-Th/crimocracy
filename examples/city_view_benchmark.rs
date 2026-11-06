//! Explicit CPU-side scene preparation benchmark. No graphics API or timing assertions.
//!
//! cargo run --locked --example city_view_benchmark

use crimocracy::AppState;
use crimocracy::city_view::{Bounds2, CityScene, DrawDetail, GPU_INSTANCE_STRIDE, GpuCityChunk};
use crimocracy::world::world_system::{insert_business, insert_neighborhood};
use crimocracy::world::{
    BusinessDraft, BusinessKind, BusinessOwner, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

const DISTRICT_COUNT: usize = 64;

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture value within bounds")
}

fn main() -> Result<(), Box<dyn Error>> {
    let registry = crimocracy::build_registry();
    let mut state = AppState::new(0xC0FFEE);
    let mut disclosed = BTreeSet::new();
    for index in 0..DISTRICT_COUNT {
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: format!("Ward {index}"),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(25 + (index % 70) as u8),
                        commercial_activity: rating(30 + (index % 65) as u8),
                        illicit_demand: rating(50),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(55),
                    },
                },
            },
        )?;
        for (number, kind) in [
            BusinessKind::Retail,
            BusinessKind::Warehouse,
            BusinessKind::Nightclub,
            BusinessKind::Transportation,
        ]
        .into_iter()
        .enumerate()
        {
            let id = insert_business(
                &registry,
                &mut state,
                BusinessDraft {
                    name: format!("Business {index}-{number}"),
                    kind,
                    functions: BTreeSet::new(),
                    neighborhood,
                    owner: BusinessOwner::Independent,
                },
            )?;
            disclosed.insert(id);
        }
    }

    let generation_start = Instant::now();
    let city = CityScene::generate(state.world(), 1991)?;
    let generation = generation_start.elapsed();

    let upload_start = Instant::now();
    let uploads: Vec<_> = city
        .districts
        .iter()
        .map(|district| GpuCityChunk::encode(district, &disclosed))
        .collect();
    let upload = upload_start.elapsed();
    let encoded_bytes: usize = uploads
        .iter()
        .map(|chunk| {
            chunk.streets.len()
                + chunk
                    .buildings
                    .iter()
                    .map(|batch| batch.bytes.len())
                    .sum::<usize>()
        })
        .sum();
    let count: usize = city
        .districts
        .iter()
        .map(|chunk| {
            chunk
                .buildings
                .iter()
                .map(|batch| batch.instances.len())
                .sum::<usize>()
        })
        .sum();

    let close_view = Bounds2 {
        min_x: 30.0,
        min_z: 30.0,
        max_x: 60.0,
        max_z: 60.0,
    };
    let draw_start = Instant::now();
    let mut plan = None;
    for _ in 0..10_000 {
        plan = Some(black_box(
            city.draw_plan(black_box(close_view), DrawDetail::Sector),
        ));
    }
    let draw = draw_start.elapsed();
    let plan = plan.expect("draw iteration must execute");
    assert_eq!(count, DISTRICT_COUNT * 1024);
    assert_eq!(plan.building_instances, 64);
    assert!(encoded_bytes >= count * GPU_INSTANCE_STRIDE);
    println!("districts: {DISTRICT_COUNT}, building instances: {count}");
    println!(
        "scene generation: {:.2} ms",
        generation.as_secs_f64() * 1000.0
    );
    println!(
        "GPU buffer encoding: {:.2} ms",
        upload.as_secs_f64() * 1000.0
    );
    println!("GPU upload payload: {encoded_bytes} bytes");
    println!(
        "10,000 close-view draw plans: {:.2} ms, {} commands, {} building instances each",
        draw.as_secs_f64() * 1000.0,
        plan.commands.len(),
        plan.building_instances
    );
    Ok(())
}
