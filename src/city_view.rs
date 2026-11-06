//! Read-only, deterministic, chunked 3D city projection for a future GUI adapter.
//!
//! This module never advances simulation time, consumes its RNG, or owns gameplay truth.
//! The caller supplies a stable visual seed per campaign. Each neighborhood is one
//! independently cullable chunk; each mesh class has one contiguous instance buffer.
//! Instance pick IDs refer to persistent businesses, not transient array positions.

use crate::core::id::{BusinessId, NeighborhoodId};
use crate::world::{BusinessKind, NeighborhoodProfile, NeighborhoodRecord, WorldState};
use std::collections::BTreeMap;
use thiserror::Error;

mod draw;
mod gpu;
pub use draw::{CityDrawCommand, CityDrawPlan, DrawDetail, DrawTarget};
pub use gpu::{GPU_INSTANCE_STRIDE, GpuCityChunk, GpuDrawBatch, GpuInstance};

pub const LOTS_PER_SIDE: usize = 32;
pub const LOT_CAPACITY: usize = LOTS_PER_SIDE * LOTS_PER_SIDE;
pub const SECTOR_SIDE_LOTS: usize = 8;
pub const SECTORS_PER_SIDE: usize = LOTS_PER_SIDE / SECTOR_SIDE_LOTS;
pub const LOT_PITCH: f32 = 22.0;
pub const DISTRICT_WIDTH: f32 = LOTS_PER_SIDE as f32 * LOT_PITCH;
pub const DISTRICT_PITCH: f32 = DISTRICT_WIDTH + 24.0;
const DISTRICTS_PER_ROW: u32 = 8;
const ARTERIAL_WIDTH: f32 = 10.0;
const ALLEY_WIDTH: f32 = 3.0;
const SIDEWALK_SETBACK: f32 = 1.0;
const BLOCK_SIDE_IN_LOTS: usize = 4;

/// Renderable geometry is a unit box (or a replacement mesh selected by the adapter).
/// A renderer can upload the instance arrays once and draw one batch per mesh per
/// visible district. No GPU runtime dependency belongs in the simulation crate.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingMesh {
    Residential,
    Storefront,
    Office,
    Industrial,
    Leisure,
    Garage,
}

impl BuildingMesh {
    const ALL: [Self; 6] = [
        Self::Residential,
        Self::Storefront,
        Self::Office,
        Self::Industrial,
        Self::Leisure,
        Self::Garage,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceTransform {
    /// Center of the mesh in meters. Y is height above the ground plane.
    pub translation: [f32; 3],
    /// Box dimensions in meters; meshes use a unit-size basis.
    pub scale: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingInstance {
    pub transform: InstanceTransform,
    /// Stable visual variation; does not encode hidden game state.
    pub variation: u16,
    /// Zero for decorative buildings; business ID for interactive buildings.
    /// This channel is for the building-picking pass only.
    pub pick_id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildingBatch {
    pub mesh: BuildingMesh,
    pub instances: Vec<BuildingInstance>,
}

/// An instance range in a district's per-mesh GPU buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InstanceRange {
    pub first: u32,
    pub count: u32,
}

/// A cullable eight-by-eight block group. Ranges index the shared district
/// building buffers, so near-camera culling does not duplicate GPU data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LotSector {
    pub bounds: Bounds2,
    pub building_ranges: [InstanceRange; 6],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2 {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

impl Bounds2 {
    pub fn is_valid(self) -> bool {
        self.min_x.is_finite()
            && self.min_z.is_finite()
            && self.max_x.is_finite()
            && self.max_z.is_finite()
            && self.min_x <= self.max_x
            && self.min_z <= self.max_z
    }

    pub fn intersects(self, other: Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_z <= other.max_z
            && self.max_z >= other.min_z
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DistrictChunk {
    pub neighborhood: NeighborhoodId,
    /// Conservative footprint for district-level frustum/viewport culling.
    pub bounds: Bounds2,
    /// Unit-box strips for streets, compatible with a single instanced draw.
    pub streets: Vec<InstanceTransform>,
    /// One contiguous upload per mesh class, in stable class order.
    pub buildings: Vec<BuildingBatch>,
    /// Sixteen independently cullable block groups in row-major order.
    pub sectors: Vec<LotSector>,
}

impl DistrictChunk {
    pub fn visible_sectors(&self, viewport: Bounds2) -> impl Iterator<Item = &LotSector> {
        self.sectors
            .iter()
            .filter(move |sector| sector.bounds.intersects(viewport))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusinessPin {
    pub neighborhood: NeighborhoodId,
    /// Ground-level world-space coordinates for focus, labels and dossier anchors.
    pub position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CityScene {
    pub districts: Vec<DistrictChunk>,
    business_pins: BTreeMap<BusinessId, BusinessPin>,
    visual_seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CityViewError {
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("neighborhood {neighborhood} has {businesses} businesses but only {capacity} lots")]
    DistrictCapacity {
        neighborhood: NeighborhoodId,
        businesses: usize,
        capacity: usize,
    },
}

impl CityScene {
    /// Rebuilds the spatial projection from authoritative world records in linear
    /// time in the number of districts and lots, plus ordered business lookups.
    /// A new business takes an unoccupied lot without shifting older businesses.
    /// Layout seeds are presentation data and must stay fixed for a campaign.
    pub fn generate(world: &WorldState, visual_seed: u64) -> Result<Self, CityViewError> {
        let mut districts = Vec::new();
        let mut business_pins = BTreeMap::new();
        for neighborhood in world.neighborhoods() {
            let (district, pins) = Self::generate_district(world, neighborhood, visual_seed)?;
            districts.push(district);
            business_pins.extend(pins);
        }
        Ok(Self {
            districts,
            business_pins,
            visual_seed,
        })
    }

    /// Regenerates only one changed neighborhood. All other district buffers and
    /// pick mappings remain intact. A new neighborhood is inserted in ID order.
    /// Failed generation leaves the previous scene completely unchanged.
    pub fn refresh_neighborhood(
        &mut self,
        world: &WorldState,
        neighborhood: NeighborhoodId,
    ) -> Result<(), CityViewError> {
        let record = world
            .get_neighborhood(neighborhood)
            .ok_or(CityViewError::MissingNeighborhood(neighborhood))?;
        let (replacement, pins) = Self::generate_district(world, record, self.visual_seed)?;
        match self
            .districts
            .binary_search_by_key(&neighborhood, |chunk| chunk.neighborhood)
        {
            Ok(index) => {
                for batch in &self.districts[index].buildings {
                    for instance in &batch.instances {
                        if instance.pick_id != 0 {
                            self.business_pins
                                .remove(&BusinessId::from_raw(instance.pick_id));
                        }
                    }
                }
                self.districts[index] = replacement;
            }
            Err(index) => self.districts.insert(index, replacement),
        }
        self.business_pins.extend(pins);
        Ok(())
    }

    fn generate_district(
        world: &WorldState,
        neighborhood: &NeighborhoodRecord,
        visual_seed: u64,
    ) -> Result<(DistrictChunk, BTreeMap<BusinessId, BusinessPin>), CityViewError> {
        let mut business_pins = BTreeMap::new();
        let id = neighborhood.id();
        let district_index = id.raw() - 1;
        let origin_x = (district_index % DISTRICTS_PER_ROW) as f32 * DISTRICT_PITCH;
        let origin_z = (district_index / DISTRICTS_PER_ROW) as f32 * DISTRICT_PITCH;
        let district_seed =
            mix64(visual_seed ^ u64::from(id.raw()).wrapping_mul(0x9E3779B97F4A7C15));
        let mut north_south_roads = [0.0f32; LOTS_PER_SIDE + 1];
        let mut east_west_roads = [0.0f32; LOTS_PER_SIDE + 1];
        for n in 0..=LOTS_PER_SIDE {
            north_south_roads[n] = road_width(district_seed, n, 0x6E73);
            east_west_roads[n] = road_width(district_seed, n, 0x6577);
        }
        // An odd stride permutes all 1024 slots without collisions.
        let stride = (((district_seed >> 16) as usize) | 1) & (LOT_CAPACITY - 1);
        let shift = district_seed as usize & (LOT_CAPACITY - 1);
        let mut businesses = vec![None; LOT_CAPACITY];
        let mut total_businesses = 0usize;
        for (ordinal, business) in world.businesses_in_neighborhood(id).enumerate() {
            total_businesses += 1;
            if ordinal < LOT_CAPACITY {
                let slot = (ordinal * stride + shift) & (LOT_CAPACITY - 1);
                businesses[slot] = Some(business);
            }
        }
        if total_businesses > LOT_CAPACITY {
            return Err(CityViewError::DistrictCapacity {
                neighborhood: id,
                businesses: total_businesses,
                capacity: LOT_CAPACITY,
            });
        }

        let mut batches: Vec<_> = BuildingMesh::ALL
            .iter()
            .map(|&mesh| BuildingBatch {
                mesh,
                instances: Vec::new(),
            })
            .collect();
        let mut sectors = Vec::with_capacity(SECTORS_PER_SIDE * SECTORS_PER_SIDE);
        for sector_row in 0..SECTORS_PER_SIDE {
            for sector_col in 0..SECTORS_PER_SIDE {
                let first: [u32; 6] =
                    std::array::from_fn(|index| batches[index].instances.len() as u32);
                for local_row in 0..SECTOR_SIDE_LOTS {
                    for local_col in 0..SECTOR_SIDE_LOTS {
                        let col = sector_col * SECTOR_SIDE_LOTS + local_col;
                        let row = sector_row * SECTOR_SIDE_LOTS + local_row;
                        let slot = row * LOTS_PER_SIDE + col;
                        let business = businesses[slot];
                        let x = origin_x + (col as f32 + 0.5) * LOT_PITCH;
                        let z = origin_z + (row as f32 + 0.5) * LOT_PITCH;
                        let variation =
                            mix64(district_seed ^ (slot as u64).wrapping_mul(0xD1B54A32D192ED03));
                        let mesh = match business {
                            Some(record) => business_mesh(record.kind()),
                            None => scenery_mesh(neighborhood.profile(), variation),
                        };
                        let height = building_height(mesh, neighborhood.profile(), variation);
                        let desired_width = 10.0 + (variation & 7).min(5) as f32;
                        let desired_depth = 10.0 + ((variation >> 3) & 7).min(5) as f32;
                        let footprint_x = desired_width.min(
                            LOT_PITCH
                                - north_south_roads[col].max(north_south_roads[col + 1])
                                - 2.0 * SIDEWALK_SETBACK,
                        );
                        let footprint_z = desired_depth.min(
                            LOT_PITCH
                                - east_west_roads[row].max(east_west_roads[row + 1])
                                - 2.0 * SIDEWALK_SETBACK,
                        );
                        let transform = InstanceTransform {
                            translation: [x, height * 0.5, z],
                            scale: [footprint_x, height, footprint_z],
                        };
                        let pick_id = business.map_or(0, |record| {
                            business_pins.insert(
                                record.id(),
                                BusinessPin {
                                    neighborhood: id,
                                    position: [x, 0.0, z],
                                },
                            );
                            record.id().raw()
                        });
                        batches[mesh as usize].instances.push(BuildingInstance {
                            transform,
                            variation: (variation >> 48) as u16,
                            pick_id,
                        });
                    }
                }
                let building_ranges = std::array::from_fn(|index| InstanceRange {
                    first: first[index],
                    count: batches[index].instances.len() as u32 - first[index],
                });
                let min_x = origin_x + (sector_col * SECTOR_SIDE_LOTS) as f32 * LOT_PITCH;
                let min_z = origin_z + (sector_row * SECTOR_SIDE_LOTS) as f32 * LOT_PITCH;
                sectors.push(LotSector {
                    bounds: Bounds2 {
                        min_x,
                        min_z,
                        max_x: min_x + SECTOR_SIDE_LOTS as f32 * LOT_PITCH,
                        max_z: min_z + SECTOR_SIDE_LOTS as f32 * LOT_PITCH,
                    },
                    building_ranges,
                });
            }
        }

        let mut streets = Vec::with_capacity((LOTS_PER_SIDE / BLOCK_SIDE_IN_LOTS + 1) * 2 + 16);
        for n in 0..=LOTS_PER_SIDE {
            let offset = n as f32 * LOT_PITCH;
            if north_south_roads[n] > 0.0 {
                streets.push(InstanceTransform {
                    translation: [origin_x + offset, 0.05, origin_z + DISTRICT_PITCH * 0.5],
                    scale: [north_south_roads[n], 0.1, DISTRICT_PITCH + ARTERIAL_WIDTH],
                });
            }
            if east_west_roads[n] > 0.0 {
                streets.push(InstanceTransform {
                    translation: [origin_x + DISTRICT_PITCH * 0.5, 0.05, origin_z + offset],
                    scale: [DISTRICT_PITCH + ARTERIAL_WIDTH, 0.1, east_west_roads[n]],
                });
            }
        }
        let district = DistrictChunk {
            neighborhood: id,
            bounds: Bounds2 {
                min_x: origin_x - ARTERIAL_WIDTH * 0.5,
                min_z: origin_z - ARTERIAL_WIDTH * 0.5,
                max_x: origin_x + DISTRICT_PITCH + ARTERIAL_WIDTH * 0.5,
                max_z: origin_z + DISTRICT_PITCH + ARTERIAL_WIDTH * 0.5,
            },
            streets,
            buildings: batches,
            sectors,
        };
        Ok((district, business_pins))
    }

    /// Uses neighborhood-ID row order to skip invisible northern rows via
    /// binary search. Only districts in the intersecting rows are examined.
    pub fn visible_districts(&self, viewport: Bounds2) -> impl Iterator<Item = &DistrictChunk> {
        let start = self
            .districts
            .partition_point(|district| district.bounds.max_z < viewport.min_z);
        self.districts[start..]
            .iter()
            .take_while(move |district| district.bounds.min_z <= viewport.max_z)
            .filter(move |district| district.bounds.intersects(viewport))
    }

    pub fn business_pin(&self, id: BusinessId) -> Option<BusinessPin> {
        self.business_pins.get(&id).copied()
    }

    /// Converts a business picking-pass integer back into its stable domain ID.
    /// The UI must independently decide what dossier information the player knows.
    pub fn business_from_pick_id(&self, pick_id: u32) -> Option<BusinessId> {
        (pick_id != 0)
            .then(|| BusinessId::from_raw(pick_id))
            .and_then(|id| self.business_pins.get_key_value(&id).map(|(id, _)| *id))
    }
}

fn road_width(seed: u64, boundary: usize, axis: u64) -> f32 {
    if boundary.is_multiple_of(BLOCK_SIDE_IN_LOTS) {
        ARTERIAL_WIDTH
    } else if boundary % BLOCK_SIDE_IN_LOTS == 2
        && mix64(seed ^ (boundary as u64).wrapping_mul(0xA0761D6478BD642F) ^ axis).is_multiple_of(3)
    {
        ALLEY_WIDTH
    } else {
        0.0
    }
}

fn business_mesh(kind: BusinessKind) -> BuildingMesh {
    match kind {
        BusinessKind::Retail | BusinessKind::Pawnshop | BusinessKind::Pharmacy => {
            BuildingMesh::Storefront
        }
        BusinessKind::ProfessionalServices
        | BusinessKind::FinancialServices
        | BusinessKind::NewsService => BuildingMesh::Office,
        BusinessKind::Warehouse
        | BusinessKind::Brewery
        | BusinessKind::Construction
        | BusinessKind::Wholesale
        | BusinessKind::Laundry
        | BusinessKind::Printing
        | BusinessKind::GarmentFactory
        | BusinessKind::Stevedoring => BuildingMesh::Industrial,
        BusinessKind::Hospitality
        | BusinessKind::Nightclub
        | BusinessKind::Lodging
        | BusinessKind::AthleticClub
        | BusinessKind::FuneralParlor
        | BusinessKind::SoupKitchen
        | BusinessKind::MovieTheater => BuildingMesh::Leisure,
        BusinessKind::Automotive
        | BusinessKind::Transportation
        | BusinessKind::CoinMachineDistribution
        | BusinessKind::TaxiGarage => BuildingMesh::Garage,
    }
}

fn scenery_mesh(profile: NeighborhoodProfile, noise: u64) -> BuildingMesh {
    let roll = (noise % 100) as u8;
    let commercial = profile.economy.commercial_activity.value();
    let offices = profile.economy.wealth.value();
    if roll < commercial / 3 {
        BuildingMesh::Storefront
    } else if roll < commercial / 3 + offices / 6 {
        BuildingMesh::Office
    } else if roll < commercial / 3 + offices / 6 + 10 {
        BuildingMesh::Industrial
    } else {
        BuildingMesh::Residential
    }
}

fn building_height(mesh: BuildingMesh, profile: NeighborhoodProfile, noise: u64) -> f32 {
    let floors = match mesh {
        BuildingMesh::Residential => 2 + (noise % 3) as u8,
        BuildingMesh::Storefront => 1 + (noise % 3) as u8,
        BuildingMesh::Office => 3 + (noise % 4) as u8 + profile.economy.wealth.value() / 25,
        BuildingMesh::Industrial => 2 + (noise % 2) as u8,
        BuildingMesh::Leisure => 2 + (noise % 3) as u8,
        BuildingMesh::Garage => 1 + (noise % 2) as u8,
    };
    f32::from(floors) * 3.5
}

/// SplitMix64-style avalanche with no shared RNG state and stable integer arithmetic.
fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests;
