//! Generates a self-contained, interactive WebGL2 city-view workbench.
//!
//! cargo run --locked --example city_gui_preview
//!
//! Open target/agent-output/city_preview/index.html in a WebGL2 browser.
//! This example is a deliberately public fictional business directory, not
//! the production player-knowledge adapter or an authoritative campaign save.

use crimocracy::AppState;
use crimocracy::city_view::{CityScene, GpuCityChunk};
use crimocracy::world::world_system::{insert_business, insert_neighborhood};
use crimocracy::world::{
    BusinessDraft, BusinessKind, BusinessOwner, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

const OUTPUT: &str = "target/agent-output/city_preview/index.html";
const HTML: &str = include_str!("../gui/city_preview/index.html");
const CSS: &str = include_str!("../gui/city_preview/styles.css");
const JAVASCRIPT: &str = include_str!("../gui/city_preview/app.js");

const WARDS: [&str; 8] = [
    "Waterfront",
    "Old Town",
    "Financial Quarter",
    "Iron Ward",
    "Market District",
    "North End",
    "Garden Ward",
    "Station Quarter",
];

const BUSINESSES: [(&str, BusinessKind, &str); 12] = [
    ("Adler & Sons", BusinessKind::Retail, "Retail provisions"),
    ("Barr's Pawn", BusinessKind::Pawnshop, "Pawnbroker"),
    (
        "Hudson Freight",
        BusinessKind::Stevedoring,
        "Dock labor and cargo",
    ),
    ("Bell Motor Works", BusinessKind::Automotive, "Motor repair"),
    ("The Marlowe", BusinessKind::Lodging, "Hotel and lodging"),
    (
        "Mercer Finance",
        BusinessKind::FinancialServices,
        "Financial services",
    ),
    (
        "Dover Printing",
        BusinessKind::Printing,
        "Commercial printer",
    ),
    (
        "Arcadia Picture House",
        BusinessKind::MovieTheater,
        "Motion picture theater",
    ),
    ("The Amber Room", BusinessKind::Nightclub, "Nightclub"),
    ("Union Storage", BusinessKind::Warehouse, "Warehouse"),
    ("Grafton Pharmacy", BusinessKind::Pharmacy, "Pharmacy"),
    (
        "Hollis Transport",
        BusinessKind::Transportation,
        "Transport and freight",
    ),
];

#[derive(Serialize)]
struct ExportedBatch {
    mesh: u8,
    instance_count: u32,
    bytes: String,
}

#[derive(Serialize)]
struct ExportedSector {
    bounds: [f32; 4],
    ranges: [[u32; 2]; 6],
}

#[derive(Serialize)]
struct ExportedDistrict {
    id: u32,
    name: String,
    bounds: [f32; 4],
    streets: String,
    street_count: u32,
    buildings: Vec<ExportedBatch>,
    sectors: Vec<ExportedSector>,
}

#[derive(Serialize)]
struct PublicDossier {
    id: u32,
    name: String,
    kind: &'static str,
    ward: &'static str,
    ward_id: u32,
    pin: [f32; 3],
    /// Visible mesh roof position for labels and accurate click testing.
    roof_height: f32,
}

#[derive(Serialize)]
struct PreviewData {
    districts: Vec<ExportedDistrict>,
    dossiers: Vec<PublicDossier>,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("valid fixture rating")
}

fn make_preview() -> Result<PreviewData, Box<dyn Error>> {
    let registry = crimocracy::build_registry();
    let mut state = AppState::new(1931);
    let mut public_directory = Vec::new();

    for (index, &ward_name) in WARDS.iter().enumerate() {
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: ward_name.to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(35 + ((index * 9) % 55) as u8),
                        commercial_activity: rating(45 + ((index * 13) % 50) as u8),
                        illicit_demand: rating(50),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(45),
                    },
                },
            },
        )?;
        for &(short_name, kind, category) in &BUSINESSES {
            let name = format!("{short_name} · {ward_name}");
            let id = insert_business(
                &registry,
                &mut state,
                BusinessDraft {
                    name: name.clone(),
                    kind,
                    functions: BTreeSet::new(),
                    neighborhood,
                    owner: BusinessOwner::Independent,
                },
            )?;
            public_directory.push((id, name, category, ward_name, neighborhood));
        }
    }

    let city = CityScene::generate(state.world(), 0x1931_B0C5)?;
    let visual_roofs: BTreeMap<u32, f32> = city
        .districts
        .iter()
        .flat_map(|district| district.buildings.iter())
        .flat_map(|batch| batch.instances.iter())
        .filter(|instance| instance.pick_id != 0)
        .map(|instance| {
            (
                instance.pick_id,
                instance.transform.translation[1] + instance.transform.scale[1] / 2.0,
            )
        })
        .collect();
    let districts = city
        .districts
        .iter()
        .map(|chunk| {
            let gpu = GpuCityChunk::encode(chunk);
            let name = state
                .world()
                .get_neighborhood(chunk.neighborhood)
                .expect("generated chunk is linked to an existing neighborhood")
                .name()
                .to_owned();
            ExportedDistrict {
                id: chunk.neighborhood.raw(),
                name,
                bounds: [
                    chunk.bounds.min_x,
                    chunk.bounds.min_z,
                    chunk.bounds.max_x,
                    chunk.bounds.max_z,
                ],
                streets: encode_base64(&gpu.streets),
                street_count: gpu.street_count,
                buildings: gpu
                    .buildings
                    .into_iter()
                    .map(|batch| ExportedBatch {
                        mesh: batch.mesh as u8,
                        instance_count: batch.instance_count,
                        bytes: encode_base64(&batch.bytes),
                    })
                    .collect(),
                sectors: chunk
                    .sectors
                    .iter()
                    .map(|sector| ExportedSector {
                        bounds: [
                            sector.bounds.min_x,
                            sector.bounds.min_z,
                            sector.bounds.max_x,
                            sector.bounds.max_z,
                        ],
                        ranges: sector
                            .building_ranges
                            .map(|range| [range.first, range.count]),
                    })
                    .collect(),
            }
        })
        .collect();
    let dossiers = public_directory
        .into_iter()
        .map(|(id, name, kind, ward, neighborhood)| {
            let pin = city
                .business_pin(id)
                .expect("generated scene must contain every modeled business");
            assert_eq!(pin.neighborhood, neighborhood);
            PublicDossier {
                id: id.raw(),
                name,
                kind,
                ward,
                ward_id: neighborhood.raw(),
                pin: pin.position,
                roof_height: *visual_roofs
                    .get(&id.raw())
                    .expect("every public property has a visible mesh"),
            }
        })
        .collect();

    Ok(PreviewData {
        districts,
        dossiers,
    })
}

fn encode_base64(bytes: &[u8]) -> String {
    const SYMBOLS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let a = group[0];
        let b = group.get(1).copied().unwrap_or(0);
        let c = group.get(2).copied().unwrap_or(0);
        encoded.push(char::from(SYMBOLS[(a >> 2) as usize]));
        encoded.push(char::from(SYMBOLS[((a & 3) << 4 | b >> 4) as usize]));
        if group.len() >= 2 {
            encoded.push(char::from(SYMBOLS[((b & 15) << 2 | c >> 6) as usize]));
        } else {
            encoded.push('=');
        }
        if group.len() == 3 {
            encoded.push(char::from(SYMBOLS[(c & 63) as usize]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

fn render_html(data: &PreviewData) -> Result<String, serde_json::Error> {
    // A literal '<' could terminate an application/json script element even
    // when it appears inside a quoted JSON string. Escape at the HTML boundary.
    let embedded_json = serde_json::to_string(data)?.replace('<', "\\u003c");
    Ok(HTML
        .replace(
            "<!-- CITY_PREVIEW_STYLES -->",
            &format!("<style>\n{CSS}\n</style>"),
        )
        .replace(
            "<!-- CITY_PREVIEW_DATA -->",
            &format!(
                "<script id=\"city-preview-data\" type=\"application/json\">{embedded_json}</script>"
            ),
        )
        .replace(
            "<!-- CITY_PREVIEW_SCRIPT -->",
            &format!("<script>\n{JAVASCRIPT}\n</script>"),
        ))
}

fn main() -> Result<(), Box<dyn Error>> {
    let data = make_preview()?;
    let html = render_html(&data)?;
    let output = Path::new(OUTPUT);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, html)?;
    eprintln!(
        "Wrote interactive WebGL2 city preview: {OUTPUT} ({} districts, {} public property dossiers)",
        data.districts.len(),
        data.dossiers.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encoding_matches_padded_and_unpadded_reference_cases() {
        for (input, expected) in [
            ("" as &str, ""),
            ("M", "TQ=="),
            ("Ma", "TWE="),
            ("Man", "TWFu"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode_base64(input.as_bytes()), expected);
        }
        assert_eq!(encode_base64(&[0, 255, 128]), "AP+A");
    }

    #[test]
    fn exported_preview_exposes_only_public_directory_fields() {
        let preview = make_preview().expect("canonical world fixture");
        assert_eq!(preview.districts.len(), WARDS.len());
        assert_eq!(preview.dossiers.len(), WARDS.len() * BUSINESSES.len());
        for district in &preview.districts {
            assert_eq!(district.sectors.len(), 16);
            assert!(district.street_count > 0);
            assert!(
                district
                    .buildings
                    .iter()
                    .all(|batch| batch.instance_count > 0)
            );
        }
        let html = render_html(&preview).expect("valid public export");
        assert!(html.contains("drawArraysInstanced"));
        assert!(html.contains("Published business directory"));
        assert!(!html.contains("business_owner"));
        assert!(!html.contains("police_presence"));
        assert!(!html.contains("illicit_demand"));
        assert!(!html.contains("<!-- CITY_PREVIEW_DATA -->"));
    }

    #[test]
    fn embedded_json_cannot_close_its_script_element() {
        let malicious = PreviewData {
            districts: Vec::new(),
            dossiers: vec![PublicDossier {
                id: 1,
                name: "</script><script>alert('x')</script>".to_owned(),
                kind: "Retail",
                ward: "Test",
                ward_id: 1,
                pin: [0.0, 0.0, 0.0],
                roof_height: 3.5,
            }],
        };
        let html = render_html(&malicious).unwrap();
        assert!(!html.contains("</script><script>alert"));
        assert!(html.contains("\\u003c/script>"));
    }
}
