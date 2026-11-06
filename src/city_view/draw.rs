//! Read-only draw commands referencing cached per-district GPU instance buffers.
//! Camera choice and actual GPU submission remain outside the simulation crate.

use super::{Bounds2, BuildingMesh, CityScene, InstanceRange};
use crate::core::id::NeighborhoodId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawDetail {
    /// Distant city view: one full draw per populated building class per district.
    District,
    /// Nearby view: only visible 8-by-8-lot sectors, with adjacent ranges merged.
    Sector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTarget {
    Streets,
    Building(BuildingMesh),
}

/// Each command refers to one instance range in the cached GPU district.
/// Rendering can submit the same range in the normal and picking passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CityDrawCommand {
    pub neighborhood: NeighborhoodId,
    pub target: DrawTarget,
    pub first_instance: u32,
    pub instance_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CityDrawPlan {
    pub commands: Vec<CityDrawCommand>,
    pub building_instances: usize,
}

impl CityScene {
    /// Construct a short draw list without copying or encoding any instance
    /// geometry. The adapter uploads each GpuCityChunk once, then uses the
    /// chosen plan to submit cached instance ranges per frame.
    pub fn draw_plan(&self, viewport: Bounds2, detail: DrawDetail) -> CityDrawPlan {
        let mut plan = CityDrawPlan::default();
        for district in self.visible_districts(viewport) {
            let neighborhood = district.neighborhood;
            if !district.streets.is_empty() {
                plan.commands.push(CityDrawCommand {
                    neighborhood,
                    target: DrawTarget::Streets,
                    first_instance: 0,
                    instance_count: district.streets.len() as u32,
                });
            }
            for (index, batch) in district.buildings.iter().enumerate() {
                let target = DrawTarget::Building(batch.mesh);
                match detail {
                    DrawDetail::District => {
                        push_merged(
                            &mut plan,
                            neighborhood,
                            target,
                            InstanceRange {
                                first: 0,
                                count: batch.instances.len() as u32,
                            },
                        );
                    }
                    DrawDetail::Sector => {
                        for sector in district.visible_sectors(viewport) {
                            push_merged(
                                &mut plan,
                                neighborhood,
                                target,
                                sector.building_ranges[index],
                            );
                        }
                    }
                }
            }
        }
        plan
    }
}

fn push_merged(
    plan: &mut CityDrawPlan,
    neighborhood: NeighborhoodId,
    target: DrawTarget,
    range: InstanceRange,
) {
    if range.count == 0 {
        return;
    }
    plan.building_instances += range.count as usize;
    if let Some(previous) = plan.commands.last_mut()
        && previous.neighborhood == neighborhood
        && previous.target == target
        && previous.first_instance + previous.instance_count == range.first
    {
        previous.instance_count += range.count;
        return;
    }
    plan.commands.push(CityDrawCommand {
        neighborhood,
        target,
        first_instance: range.first,
        instance_count: range.count,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::AppState;
    use crate::world::world_system::insert_neighborhood;
    use crate::world::{
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, Rating,
    };

    #[test]
    fn sector_draw_plan_draws_one_block_without_reencoding_any_instances() {
        let mut state = AppState::new(112);
        insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Old Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).unwrap(),
                        commercial_activity: Rating::try_new(65).unwrap(),
                        illicit_demand: Rating::try_new(50).unwrap(),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(50).unwrap(),
                    },
                },
            },
        )
        .unwrap();
        let city = CityScene::generate(state.world(), 23).unwrap();
        let near = Bounds2 {
            min_x: 30.0,
            min_z: 30.0,
            max_x: 60.0,
            max_z: 60.0,
        };
        let close = city.draw_plan(near, DrawDetail::Sector);
        let distant = city.draw_plan(near, DrawDetail::District);
        assert_eq!(close.building_instances, 64);
        assert_eq!(distant.building_instances, 1024);
        assert!(close.commands.len() <= 7);
        assert!(
            close
                .commands
                .iter()
                .any(|command| command.target == DrawTarget::Streets)
        );
        assert_eq!(
            close
                .commands
                .iter()
                .filter(|command| matches!(command.target, DrawTarget::Building(_)))
                .map(|command| command.instance_count as usize)
                .sum::<usize>(),
            close.building_instances
        );
    }

    #[test]
    fn full_sector_coverage_merges_back_into_district_draw_commands() {
        let mut state = AppState::new(113);
        insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Central".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).unwrap(),
                        commercial_activity: Rating::try_new(65).unwrap(),
                        illicit_demand: Rating::try_new(50).unwrap(),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(50).unwrap(),
                    },
                },
            },
        )
        .unwrap();
        let city = CityScene::generate(state.world(), 24).unwrap();
        let all = Bounds2 {
            min_x: -10.0,
            min_z: -10.0,
            max_x: 800.0,
            max_z: 800.0,
        };
        assert_eq!(
            city.draw_plan(all, DrawDetail::Sector),
            city.draw_plan(all, DrawDetail::District)
        );
        assert_eq!(
            city.draw_plan(
                Bounds2 {
                    min_x: 300.0,
                    min_z: 300.0,
                    max_x: 250.0,
                    max_z: 250.0,
                },
                DrawDetail::Sector,
            ),
            CityDrawPlan::default()
        );
    }
}
