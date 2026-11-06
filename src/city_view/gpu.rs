//! Renderer-independent instance-buffer encoding for a graphics adapter.
//!
//! The wire format is explicitly little-endian, not a transmute of a Rust
//! struct. An adapter can upload the returned bytes directly as an instance
//! vertex buffer without unsafe code or a graphics dependency in the core.

use super::{BuildingMesh, DistrictChunk, InstanceTransform};
use crate::core::id::{BusinessId, NeighborhoodId};
use std::collections::BTreeSet;

/// Six little-endian f32 values followed by two little-endian u32 values.
/// Shader attributes, in byte order: translation.xyz, scale.xyz, pick ID,
/// variation. Use 32-byte stride with per-instance step mode.
pub const GPU_INSTANCE_STRIDE: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuInstance {
    pub translation: [f32; 3],
    pub scale: [f32; 3],
    pub pick_id: u32,
    pub variation: u32,
}

impl GpuInstance {
    /// Encodes the explicit attribute layout without relying on host padding,
    /// host endianness, or unsafe type casts.
    pub fn encode(self) -> [u8; GPU_INSTANCE_STRIDE] {
        let mut bytes = [0u8; GPU_INSTANCE_STRIDE];
        for (index, component) in self.translation.into_iter().chain(self.scale).enumerate() {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&component.to_le_bytes());
        }
        bytes[24..28].copy_from_slice(&self.pick_id.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.variation.to_le_bytes());
        bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuDrawBatch {
    pub mesh: BuildingMesh,
    pub instance_count: u32,
    /// Contiguous encoded 32-byte instance records for one mesh/material.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuCityChunk {
    pub neighborhood: NeighborhoodId,
    pub street_count: u32,
    /// Roads use the same instance layout and a unit-cube road mesh.
    pub streets: Vec<u8>,
    /// Empty geometry classes are omitted, avoiding zero-length GPU uploads.
    pub buildings: Vec<GpuDrawBatch>,
}

impl GpuCityChunk {
    /// Prepare only this district for upload. Cache the result in the adapter
    /// and repeat only after refresh_neighborhood replaces this district.
    ///
    /// Only explicitly disclosed business identities survive encoding. Raw
    /// GPU vertex data can be inspected by players, so a browser-side dossier
    /// lookup filter is not an adequate substitute for this disclosure gate.
    /// The caller must derive this set from authorized player-visible knowledge.
    pub fn encode(district: &DistrictChunk, disclosed: &BTreeSet<BusinessId>) -> Self {
        let mut streets = Vec::with_capacity(district.streets.len() * GPU_INSTANCE_STRIDE);
        for &transform in &district.streets {
            push_instance(&mut streets, transform, 0, 0);
        }

        let buildings = district
            .buildings
            .iter()
            .filter(|batch| !batch.instances.is_empty())
            .map(|batch| {
                let mut bytes = Vec::with_capacity(batch.instances.len() * GPU_INSTANCE_STRIDE);
                for instance in &batch.instances {
                    let pick_id = if instance.pick_id != 0
                        && disclosed.contains(&BusinessId::from_raw(instance.pick_id))
                    {
                        instance.pick_id
                    } else {
                        0
                    };
                    push_instance(
                        &mut bytes,
                        instance.transform,
                        pick_id,
                        u32::from(instance.variation),
                    );
                }
                GpuDrawBatch {
                    mesh: batch.mesh,
                    instance_count: batch.instances.len() as u32,
                    bytes,
                }
            })
            .collect();
        Self {
            neighborhood: district.neighborhood,
            street_count: district.streets.len() as u32,
            streets,
            buildings,
        }
    }
}

fn push_instance(
    destination: &mut Vec<u8>,
    transform: InstanceTransform,
    pick_id: u32,
    variation: u32,
) {
    destination.extend_from_slice(
        &GpuInstance {
            translation: transform.translation,
            scale: transform.scale,
            pick_id,
            variation,
        }
        .encode(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city_view::{Bounds2, BuildingBatch, BuildingInstance};
    use crate::core::id::NeighborhoodId;

    #[test]
    fn gpu_record_has_documented_32_byte_endian_safe_layout() {
        assert_eq!(std::mem::size_of::<GpuInstance>(), GPU_INSTANCE_STRIDE);
        let input = GpuInstance {
            translation: [1.5, -2.25, 3.0],
            scale: [4.0, 5.0, 6.0],
            pick_id: 0x1234_5678,
            variation: 0xAABB_CCDD,
        };
        let bytes = input.encode();
        for (index, expected) in input
            .translation
            .iter()
            .chain(input.scale.iter())
            .enumerate()
        {
            let start = index * 4;
            assert_eq!(&bytes[start..start + 4], &expected.to_le_bytes());
        }
        assert_eq!(&bytes[24..28], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&bytes[28..32], &[0xDD, 0xCC, 0xBB, 0xAA]);
    }

    #[test]
    fn gpu_chunk_retains_pick_identity_and_skips_empty_batches() {
        let transform = InstanceTransform {
            translation: [20.0, 5.0, 40.0],
            scale: [12.0, 10.0, 14.0],
        };
        let neighborhood = NeighborhoodId::from_raw(3);
        let chunk = DistrictChunk {
            neighborhood,
            bounds: Bounds2 {
                min_x: 0.0,
                min_z: 0.0,
                max_x: 100.0,
                max_z: 100.0,
            },
            streets: vec![transform],
            buildings: vec![
                BuildingBatch {
                    mesh: BuildingMesh::Residential,
                    instances: Vec::new(),
                },
                BuildingBatch {
                    mesh: BuildingMesh::Office,
                    instances: vec![
                        BuildingInstance {
                            transform,
                            pick_id: 31415,
                            variation: 42,
                        },
                        BuildingInstance {
                            transform,
                            pick_id: 27182,
                            variation: 12,
                        },
                    ],
                },
            ],
            sectors: Vec::new(),
        };
        let disclosed = BTreeSet::from([BusinessId::from_raw(31415)]);
        let gpu = GpuCityChunk::encode(&chunk, &disclosed);
        assert_eq!(gpu.neighborhood, neighborhood);
        assert_eq!(gpu.street_count, 1);
        assert_eq!(gpu.streets.len(), GPU_INSTANCE_STRIDE);
        assert_eq!(gpu.buildings.len(), 1);
        assert_eq!(gpu.buildings[0].mesh, BuildingMesh::Office);
        assert_eq!(gpu.buildings[0].instance_count, 2);
        assert_eq!(gpu.buildings[0].bytes.len(), GPU_INSTANCE_STRIDE * 2);
        assert_eq!(&gpu.buildings[0].bytes[24..28], &31415u32.to_le_bytes());
        assert_eq!(&gpu.buildings[0].bytes[28..32], &42u32.to_le_bytes());
        assert_eq!(&gpu.buildings[0].bytes[56..60], &0u32.to_le_bytes());
        assert_eq!(&gpu.buildings[0].bytes[60..64], &12u32.to_le_bytes());
        let concealed = GpuCityChunk::encode(&chunk, &BTreeSet::new());
        assert_eq!(&concealed.buildings[0].bytes[24..28], &0u32.to_le_bytes());
        let both_disclosed = GpuCityChunk::encode(
            &chunk,
            &BTreeSet::from([BusinessId::from_raw(31415), BusinessId::from_raw(27182)]),
        );
        assert_eq!(
            &both_disclosed.buildings[0].bytes[56..60],
            &27182u32.to_le_bytes()
        );
    }
}
