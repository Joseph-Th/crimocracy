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

pub const LOTS_PER_SIDE: usize = 32;
pub const LOT_CAPACITY: usize = LOTS_PER_SIDE * LOTS_PER_SIDE;
pub const LOT_PITCH: f32 = 22.0;
pub const DISTRICT_WIDTH: f32 = LOTS_PER_SIDE as f32 * LOT_PITCH;
pub const DISTRICT_PITCH: f32 = DISTRICT_WIDTH + 96.0;
const DISTRICTS_PER_ROW: u32 = 8;
const ROAD_WIDTH: f32 = 4.0;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2 {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

impl Bounds2 {
    pub fn intersects(self, other: Self) -> bool {
        self.min_x <= other.max_x
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
        // An odd stride permutes all 1024 slots without collisions.
        let stride = (((district_seed >> 16) as usize) | 1) & (LOT_CAPACITY - 1);
        let shift = district_seed as usize & (LOT_CAPACITY - 1);
        let mut businesses = vec![None; LOT_CAPACITY];
        for (ordinal, business) in world.businesses_in_neighborhood(id).enumerate() {
            if ordinal >= LOT_CAPACITY {
                return Err(CityViewError::DistrictCapacity {
                    neighborhood: id,
                    businesses: ordinal + 1,
                    capacity: LOT_CAPACITY,
                });
            }
            let slot = (ordinal * stride + shift) & (LOT_CAPACITY - 1);
            businesses[slot] = Some(business);
        }

        let mut batches: Vec<_> = BuildingMesh::ALL
            .iter()
            .map(|&mesh| BuildingBatch {
                mesh,
                instances: Vec::new(),
            })
            .collect();
        for (slot, business) in businesses.into_iter().enumerate() {
            let col = slot % LOTS_PER_SIDE;
            let row = slot / LOTS_PER_SIDE;
            let x = origin_x + (col as f32 + 0.5) * LOT_PITCH;
            let z = origin_z + (row as f32 + 0.5) * LOT_PITCH;
            let variation = mix64(district_seed ^ (slot as u64).wrapping_mul(0xD1B54A32D192ED03));
            let mesh = match business {
                Some(record) => business_mesh(record.kind()),
                None => scenery_mesh(neighborhood.profile(), variation),
            };
            let height = building_height(mesh, neighborhood.profile(), variation);
            let transform = InstanceTransform {
                translation: [x, height * 0.5, z],
                scale: [
                    12.0 + (variation & 3) as f32,
                    height,
                    12.0 + ((variation >> 2) & 3) as f32,
                ],
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

        let mut streets = Vec::with_capacity((LOTS_PER_SIDE + 1) * 2);
        for n in 0..=LOTS_PER_SIDE {
            let offset = n as f32 * LOT_PITCH;
            streets.push(InstanceTransform {
                translation: [origin_x + offset, 0.05, origin_z + DISTRICT_WIDTH * 0.5],
                scale: [ROAD_WIDTH, 0.1, DISTRICT_WIDTH + ROAD_WIDTH],
            });
            streets.push(InstanceTransform {
                translation: [origin_x + DISTRICT_WIDTH * 0.5, 0.05, origin_z + offset],
                scale: [DISTRICT_WIDTH + ROAD_WIDTH, 0.1, ROAD_WIDTH],
            });
        }
        let district = DistrictChunk {
            neighborhood: id,
            bounds: Bounds2 {
                min_x: origin_x - ROAD_WIDTH * 0.5,
                min_z: origin_z - ROAD_WIDTH * 0.5,
                max_x: origin_x + DISTRICT_WIDTH + ROAD_WIDTH * 0.5,
                max_z: origin_z + DISTRICT_WIDTH + ROAD_WIDTH * 0.5,
            },
            streets,
            buildings: batches,
        };
        Ok((district, business_pins))
    }

    pub fn visible_districts(&self, viewport: Bounds2) -> impl Iterator<Item = &DistrictChunk> {
        self.districts
            .iter()
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
