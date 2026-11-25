//! Release-safe validation of persisted personal work history.

use super::*;
use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::core::invariants::StateValidationError;
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::time::{DAY_DURATION, SimTime};

#[test]
fn restore_rejects_impossible_career_history() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    for _ in 0..7 {
        city.state.advance_clock(DAY_DURATION);
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        apply_due_character_lives(&mut city.state);
    }
    let original = city.state.world().get_character(official).unwrap();
    let life = original.life();
    assert_eq!(life.career_days(), 6);
    let (last_at, action) = life.last_activity().unwrap();
    assert_eq!(action, CivicActivity::Politics(council));
    let save = build_save(&registry, &city.state).unwrap();
    restore_save(&registry, save.clone()).unwrap();
    let original_bytes = bincode::serialize(original).unwrap();
    let bytes = bincode::serialize(&save).unwrap();
    let positions: Vec<_> = bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    let start = positions[0];
    let rest_day = (1..=7)
        .find(|day| (*day + u64::from(official.raw())).is_multiple_of(7))
        .unwrap();
    let forgeries = [
        (
            "activity without any workdays",
            CharacterLife {
                career_days: 0,
                ..life
            },
        ),
        (
            "workdays without an activity",
            CharacterLife {
                last_activity: None,
                ..life
            },
        ),
        (
            "seven workdays in a six-shift week",
            CharacterLife {
                career_days: 7,
                ..life
            },
        ),
        (
            "working on a scheduled rest day",
            CharacterLife {
                career_days: 1,
                last_activity: Some((SimTime::from_minutes(rest_day * DAY_MINUTES), action)),
                ..life
            },
        ),
        (
            "political work for a criminal organization",
            CharacterLife {
                last_activity: Some((last_at, CivicActivity::Politics(city.player))),
                ..life
            },
        ),
    ];
    for (scenario, life) in forgeries {
        let mut replacement = original.clone();
        replacement.life = life;
        let mut forged = bytes.clone();
        forged.splice(
            start..start + original_bytes.len(),
            bincode::serialize(&replacement).unwrap(),
        );
        let envelope: SaveEnvelope = bincode::deserialize(&forged).unwrap();
        assert_eq!(
            restore_save(&registry, envelope).err(),
            Some(LoadError::InvalidState(
                StateValidationError::InvalidCharacterLife
            )),
            "{scenario}"
        );
    }
}
