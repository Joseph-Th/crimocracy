use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_tick;

fn bytes(registry: &Registry, state: &AppState) -> Vec<u8> {
    bincode::serialize(&build_save(registry, state).unwrap()).unwrap()
}

#[test]
fn generated_organizations_have_real_reporting_relationships_and_manager_contacts() {
    use crate::recruitment::recruitment_system::find_recruitment_candidates;
    let registry = crate::build_registry();
    let city = generate_city(&registry, CityScale::Small.config(12)).unwrap();
    for org in std::iter::once(city.player).chain(city.rivals.iter().copied()) {
        for member in city.state.world().characters_in_organization(org) {
            if let Some(supervisor) = member.supervisor() {
                assert!(
                    city.state
                        .social()
                        .get_relationship(member.id(), supervisor)
                        .is_some(),
                    "generated member {} lacks a relationship to the actual supervisor",
                    member.id()
                );
            }
        }
    }
    assert_eq!(
        find_recruitment_candidates(&registry, &city.state, city.player, city.boss)
            .unwrap()
            .len(),
        2
    );
    let manager = city
        .state
        .world()
        .characters_in_organization(city.player)
        .find(|member| {
            member
                .capability(C::Surveillance)
                .is_some_and(|rating| rating.value() == 91)
        })
        .unwrap()
        .id();
    let candidates =
        find_recruitment_candidates(&registry, &city.state, city.player, manager).unwrap();
    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|id| {
        city.state
            .world()
            .get_character(*id)
            .unwrap()
            .organization()
            .is_none()
    }));
}

#[test]
fn generated_infrastructure_supports_specialized_enterprise_networks() {
    let registry = crate::build_registry();
    let city = generate_city(&registry, CityScale::Large.config(516708459)).unwrap();
    let world = city.state.world();
    for (kind, required) in [
        (K::Automotive, vec![F::VehicleWorkshop, F::ResaleMarket]),
        (
            K::Construction,
            vec![F::UnionAccess, F::VehicleFleet, F::Warehousing],
        ),
        (K::GarmentFactory, vec![F::UnionAccess]),
        (
            K::Stevedoring,
            vec![
                F::DockAccess,
                F::Warehousing,
                F::UnionAccess,
                F::DistributionInfrastructure,
            ],
        ),
        (
            K::CoinMachineDistribution,
            vec![
                F::VehicleFleet,
                F::CashIntensive,
                F::CustomerAccess,
                F::DistributionInfrastructure,
            ],
        ),
        (
            K::Lodging,
            vec![
                F::Lodging,
                F::CashIntensive,
                F::CustomerAccess,
                F::MeetingSpace,
            ],
        ),
        (
            K::AthleticClub,
            vec![F::SportingVenue, F::MeetingSpace, F::CustomerAccess],
        ),
        (K::Nightclub, vec![F::Nightlife, F::CustomerAccess]),
        (
            K::Wholesale,
            vec![
                F::CustomerAccess,
                F::Warehousing,
                F::DistributionInfrastructure,
            ],
        ),
    ] {
        let records = world
            .public_business_directory()
            .filter(|(_, _, actual, _)| *actual == kind)
            .collect::<Vec<_>>();
        assert!(!records.is_empty(), "missing generated {kind:?}");
        for (id, _, _, _) in records {
            let record = world.get_business(id).unwrap();
            assert!(
                required
                    .iter()
                    .all(|function| record.has_function(*function)),
                "{} lacks its expected {kind:?} infrastructure",
                record.name()
            );
        }
    }
    let saved = bytes(&registry, &city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(saved, bytes(&registry, &restored));
}

#[test]
fn generated_small_city_has_real_owners_economies_policing_and_playable_leads() {
    let registry = crate::build_registry();
    let generated = generate_city(&registry, CityScale::Small.config(12)).unwrap();
    let state = &generated.state;
    let world = state.world();
    assert_eq!(world.neighborhoods().count(), 4);
    let addresses = world.public_business_directory().collect::<Vec<_>>();
    assert_eq!(addresses.len(), 48);
    assert_eq!(state.player_organization(), Some(generated.player));
    assert_eq!(generated.rivals.len(), 2);
    assert_eq!(generated.police.len(), 1);
    assert_eq!(
        state
            .world()
            .businesses_owned_by_organization(generated.player)
            .count(),
        2
    );
    assert_eq!(
        state
            .world()
            .characters_in_organization(generated.player)
            .count(),
        6
    );
    assert_eq!(
        state
            .enterprises()
            .enterprises_for_organization(generated.player)
            .count(),
        1
    );
    assert_eq!(
        state
            .enterprises()
            .enterprises_for_organization(generated.rivals[0])
            .count(),
        1
    );
    for (id, _, _, neighborhood) in &addresses {
        assert!(
            state.economy().get_business_economy(*id).is_some(),
            "every generated shop has real books and a settlement clock"
        );
        assert!(world.get_neighborhood(*neighborhood).is_some());
    }
    for neighborhood in world.neighborhoods() {
        assert_eq!(
            state
                .legal()
                .active_patrol_deployments_for_neighborhood(neighborhood.id())
                .count(),
            1
        );
        assert_eq!(
            crate::legal::jurisdiction_system::resolve_case_intake_authority(
                state,
                neighborhood.id()
            ),
            Some(generated.police[0])
        );
    }
    let scene = generated.scene().unwrap();
    assert_eq!(scene.districts.len(), 4);
    assert_eq!(scene, generated.scene().unwrap());
    let saved = bytes(&registry, state);
    let loaded = restore_save(&registry, build_save(&registry, state).unwrap()).unwrap();
    assert_eq!(saved, bytes(&registry, &loaded));
}

#[test]
fn generated_worlds_are_deterministic_diverse_and_independent_of_gameplay_draws() {
    let registry = crate::build_registry();
    let config = CityScale::Medium.config(0xA12F);
    let first = generate_city(&registry, config).unwrap();
    let same = generate_city(&registry, config).unwrap();
    assert_eq!(
        bytes(&registry, &first.state),
        bytes(&registry, &same.state)
    );
    assert_eq!(first.scene().unwrap(), same.scene().unwrap());
    let changed = generate_city(&registry, CityScale::Medium.config(0xA130)).unwrap();
    assert_ne!(
        bytes(&registry, &first.state),
        bytes(&registry, &changed.state)
    );
    assert_ne!(
        first.state.world().neighborhoods().next().unwrap().name(),
        changed.state.world().neighborhoods().next().unwrap().name()
    );
    assert_eq!(first.state.world().neighborhoods().count(), 8);
    assert_eq!(first.state.world().public_business_directory().count(), 144);
    assert_eq!(first.police.len(), 2);
    assert_eq!(first.rivals.len(), 3);
}

#[test]
fn many_seeds_generate_valid_cities_and_can_advance_real_time() {
    let registry = crate::build_registry();
    for seed in 0..24 {
        let mut generated = generate_city(&registry, CityScale::Small.config(seed)).unwrap();
        for _ in 0..15 {
            run_tick(&registry, &mut generated.state).unwrap();
        }
        crate::core::invariants::validate_state(&generated.state).unwrap();
        assert_eq!(generated.state.now().as_minutes(), 15);
        assert_eq!(
            bytes(&registry, &generated.state),
            bytes(
                &registry,
                &restore_save(&registry, build_save(&registry, &generated.state).unwrap()).unwrap()
            )
        );
    }
    let mut large = generate_city(&registry, CityScale::Large.config(771)).unwrap();
    assert_eq!(large.state.world().neighborhoods().count(), 16);
    assert_eq!(large.state.world().public_business_directory().count(), 384);
    assert_eq!(large.scene().unwrap().districts.len(), 16);
    for _ in 0..5 {
        run_tick(&registry, &mut large.state).unwrap();
    }
    let original = bytes(&registry, &large.state);
    let restored = restore_save(&registry, build_save(&registry, &large.state).unwrap()).unwrap();
    assert_eq!(original, bytes(&registry, &restored));
}

#[test]
fn invalid_city_budgets_are_rejected_before_creating_any_state() {
    let registry = crate::build_registry();
    for bad in [
        CityGenerationConfig {
            seed: 1,
            districts: 3,
            businesses_per_district: 12,
            rivals: 1,
        },
        CityGenerationConfig {
            seed: 1,
            districts: 4,
            businesses_per_district: 7,
            rivals: 1,
        },
        CityGenerationConfig {
            seed: 1,
            districts: 4,
            businesses_per_district: 41,
            rivals: 1,
        },
        CityGenerationConfig {
            seed: 1,
            districts: 4,
            businesses_per_district: 12,
            rivals: 4,
        },
        CityGenerationConfig {
            seed: 1,
            districts: 24,
            businesses_per_district: 12,
            rivals: 9,
        },
    ] {
        assert!(
            generate_city(&registry, bad).is_err(),
            "{bad:?} should reject"
        );
    }
}

#[test]
fn largest_supported_custom_city_has_full_police_and_business_coverage() {
    let registry = crate::build_registry();
    let config = CityGenerationConfig {
        seed: 0xC173_2026,
        districts: 24,
        businesses_per_district: 40,
        rivals: 8,
    };
    let mut city = generate_city(&registry, config).unwrap();
    assert_eq!(city.state.world().neighborhoods().count(), 24);
    assert_eq!(city.state.world().public_business_directory().count(), 960);
    assert_eq!(city.police.len(), 6);
    assert_eq!(city.rivals.len(), 8);
    let names = city
        .state
        .world()
        .neighborhoods()
        .map(|record| record.name())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names.len(),
        24,
        "all generated district names must be unique"
    );
    let kinds = city
        .state
        .world()
        .public_business_directory()
        .map(|(_, _, kind, _)| kind)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        kinds.len(),
        ALL_BUSINESS_KINDS.len(),
        "a full-scale city should sample every modeled business kind"
    );
    for ward in city.state.world().neighborhoods() {
        assert!(
            crate::legal::jurisdiction_system::resolve_case_intake_authority(
                &city.state,
                ward.id()
            )
            .is_some()
        );
    }
    let geometry = city.scene().unwrap();
    assert_eq!(geometry.districts.len(), 24);
    run_tick(&registry, &mut city.state).unwrap();
    let before = bytes(&registry, &city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(before, bytes(&registry, &restored));
}
