//! Generates an optional geometry inspection image. Not a gameplay renderer.
//!
//! cargo run --locked --example city_view_workbench
//! Open or convert the portable PPM image to inspect roads, building scale,
//! geometry overlap, and mesh variety without depending on any graphics API.

use crimocracy::AppState;
use crimocracy::city_view::{BuildingInstance, BuildingMesh, CityScene, InstanceTransform};
use crimocracy::world::world_system::{insert_business, insert_neighborhood};
use crimocracy::world::{
    ALL_BUSINESS_KINDS, BusinessDraft, BusinessOwner, NeighborhoodDraft,
    NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

const WIDTH: usize = 1600;
const HEIGHT: usize = 1000;
const BACKGROUND: [u8; 3] = [17, 22, 26];
const GROUND: [u8; 3] = [58, 63, 62];
const STREET: [u8; 3] = [30, 35, 39];
const PREVIEW_PATH: &str = "target/agent-output/city_view_workbench.ppm";

struct Image {
    pixels: Vec<[u8; 3]>,
}

impl Image {
    fn new() -> Self {
        Self {
            pixels: vec![BACKGROUND; WIDTH * HEIGHT],
        }
    }

    fn polygon(&mut self, corners: &[[f32; 2]; 4], color: [u8; 3]) {
        let min_x = corners
            .iter()
            .map(|point| point[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_x = corners
            .iter()
            .map(|point| point[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(WIDTH as f32) as usize;
        let min_y = corners
            .iter()
            .map(|point| point[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_y = corners
            .iter()
            .map(|point| point[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(HEIGHT as f32) as usize;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let mut positive = false;
                let mut negative = false;
                for index in 0..4 {
                    let a = corners[index];
                    let b = corners[(index + 1) % 4];
                    let cross = (b[0] - a[0]) * (y as f32 + 0.5 - a[1])
                        - (b[1] - a[1]) * (x as f32 + 0.5 - a[0]);
                    positive |= cross > 0.05;
                    negative |= cross < -0.05;
                }
                if !(positive && negative) {
                    self.pixels[y * WIDTH + x] = color;
                }
            }
        }
    }

    fn write_ppm(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut ppm = Vec::with_capacity(WIDTH * HEIGHT * 3 + 64);
        ppm.extend_from_slice(format!("P6\n{WIDTH} {HEIGHT}\n255\n").as_bytes());
        for pixel in &self.pixels {
            ppm.extend_from_slice(pixel);
        }
        std::fs::write(path, ppm)?;
        Ok(())
    }
}

fn project(x: f32, y: f32, z: f32) -> [f32; 2] {
    [800.0 + (x - z) * 0.96, 80.0 + (x + z) * 0.57 - y * 0.85]
}

fn shade(rgb: [u8; 3], change: i16) -> [u8; 3] {
    rgb.map(|channel| (i16::from(channel) + change).clamp(0, 255) as u8)
}

fn draw_ground_box(image: &mut Image, transform: InstanceTransform, rgb: [u8; 3]) {
    let [x, y, z] = transform.translation;
    let [width, height, depth] = transform.scale;
    let (hx, hz) = (width * 0.5, depth * 0.5);
    let low = y - height * 0.5;
    let high = y + height * 0.5;
    let top = [
        project(x - hx, high, z - hz),
        project(x + hx, high, z - hz),
        project(x + hx, high, z + hz),
        project(x - hx, high, z + hz),
    ];
    let ground = [
        project(x - hx, low, z - hz),
        project(x + hx, low, z - hz),
        project(x + hx, low, z + hz),
        project(x - hx, low, z + hz),
    ];
    image.polygon(&[top[1], top[2], ground[2], ground[1]], shade(rgb, -23));
    image.polygon(&[top[3], top[2], ground[2], ground[3]], shade(rgb, -10));
    image.polygon(&top, shade(rgb, 22));
}

fn building_color(mesh: BuildingMesh, instance: &BuildingInstance) -> [u8; 3] {
    let color = match mesh {
        BuildingMesh::Residential => [87, 104, 107],
        BuildingMesh::Storefront => [153, 130, 94],
        BuildingMesh::Office => [101, 117, 113],
        BuildingMesh::Industrial => [125, 119, 108],
        BuildingMesh::Leisure => [150, 103, 88],
        BuildingMesh::Garage => [91, 101, 109],
    };
    let brightness = i16::from((instance.variation % 13) as u8) - 6;
    shade(
        color,
        if instance.pick_id == 0 {
            brightness
        } else {
            brightness + 33
        },
    )
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("valid authored preview rating")
}

fn main() -> Result<(), Box<dyn Error>> {
    let registry = crimocracy::build_registry();
    let mut state = AppState::new(2026);
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Industrial Waterfront".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(65),
                    commercial_activity: rating(82),
                    illicit_demand: rating(55),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(45),
                },
            },
        },
    )?;
    for (index, kind) in ALL_BUSINESS_KINDS.into_iter().enumerate() {
        insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: format!("Example establishment {}", index + 1),
                kind,
                functions: BTreeSet::new(),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )?;
    }
    let scene = CityScene::generate(state.world(), 0xCAFE_BABE)?;
    let chunk = &scene.districts[0];
    let mut image = Image::new();
    draw_ground_box(
        &mut image,
        InstanceTransform {
            translation: [364.0, -0.15, 364.0],
            scale: [728.0, 0.1, 728.0],
        },
        GROUND,
    );
    for road in &chunk.streets {
        draw_ground_box(&mut image, *road, STREET);
    }
    let mut buildings: Vec<_> = chunk
        .buildings
        .iter()
        .flat_map(|batch| {
            batch
                .instances
                .iter()
                .map(move |instance| (batch.mesh, instance))
        })
        .collect();
    buildings.sort_by(|a, b| {
        let depth = |instance: &BuildingInstance| {
            instance.transform.translation[0] + instance.transform.translation[2]
        };
        depth(a.1).total_cmp(&depth(b.1))
    });
    for (mesh, instance) in buildings {
        draw_ground_box(
            &mut image,
            instance.transform,
            building_color(mesh, instance),
        );
    }
    image.write_ppm(Path::new(PREVIEW_PATH))?;
    eprintln!("Wrote {WIDTH}x{HEIGHT} city geometry preview: {PREVIEW_PATH}");
    Ok(())
}
