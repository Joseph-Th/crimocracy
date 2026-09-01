//! Focused tests for patrol deployment validation and presence resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::legal::JurisdictionDraft;
use crate::legal::jurisdiction_system::{JurisdictionError, validate_set_jurisdiction};
use crate::world::world_system::{insert_neighborhood, insert_organization};
use crate::world::{
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft,
};
use std::collections::BTreeSet;

fn make_fixture() -> (crate::Registry, AppState, OrganizationId, NeighborhoodId) {
    let registry = build_registry();
    let mut state = AppState::new(0x0A70_1933);
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Patrol Test Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(60).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("patrol neighborhood fixture should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Patrol Test Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("patrol authority fixture should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
        },
    )
    .expect("patrol jurisdiction fixture should validate")
    .commit(&mut state)
    .expect("patrol jurisdiction fixture should commit");
    (registry, state, police, neighborhood)
}

fn window(start: u16, duration: u16, presence: u8) -> PatrolWindow {
    PatrolWindow::try_new(
        DayMinute::try_new(start).expect("fixture minute should validate"),
        duration,
        Rating::try_new(presence).expect("fixture rating should validate"),
    )
    .expect("fixture patrol window should validate")
}

#[test]
fn patrol_windows_wrap_midnight_and_leave_real_coverage_gaps() {
    let (_registry, mut state, police, neighborhood) = make_fixture();
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(1_320, 240, 80), window(480, 120, 40)],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");

    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(1_380))
            .map(Rating::value),
        Some(80)
    );
    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(60)).map(Rating::value),
        Some(80)
    );
    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(300))
            .map(Rating::value),
        Some(0)
    );
    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(540))
            .map(Rating::value),
        Some(40)
    );
    validate_state(&state).expect("patrol state should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn overlapping_patrol_windows_are_rejected_without_mutation() {
    let (_registry, state, police, neighborhood) = make_fixture();
    let error = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(1_380, 120, 70), window(30, 60, 50)],
        },
    )
    .expect_err("overlapping wrapped windows must be rejected");
    assert!(matches!(error, PatrolError::OverlappingWindow { .. }));
    assert_eq!(
        state
            .legal()
            .patrol_deployments()
            .filter(|deployment| deployment.neighborhood() == neighborhood)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn active_patrol_blocks_jurisdiction_contraction_until_suspended() {
    let (_registry, mut state, police, neighborhood) = make_fixture();
    let second_neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Second Patrol Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("second neighborhood should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood, second_neighborhood]),
            case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
        },
    )
    .expect("expanded jurisdiction should validate")
    .commit(&mut state)
    .expect("expanded jurisdiction should commit");
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, 1_440, 70)],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");

    let contraction = JurisdictionDraft {
        organization: police,
        neighborhoods: BTreeSet::from([second_neighborhood]),
        case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
    };
    let error = validate_set_jurisdiction(&state, contraction.clone())
        .expect_err("active patrol must block removal of its neighborhood");
    assert_eq!(
        error,
        JurisdictionError::ActivePatrolDeployment {
            organization: police,
            neighborhood,
            deployment,
        }
    );

    validate_patrol_transition(&state, deployment, PatrolDeploymentTransition::Suspend)
        .expect("active patrol should suspend")
        .commit(&mut state)
        .expect("patrol suspension should commit");
    validate_set_jurisdiction(&state, contraction)
        .expect("suspended patrol should not block jurisdiction contraction")
        .commit(&mut state)
        .expect("jurisdiction contraction should commit");
    validate_state(&state).expect("suspended patrol may remain outside current jurisdiction");
    validate_invariants(&state);
}

#[test]
fn stale_patrol_revision_cannot_overwrite_lifecycle_change() {
    let (_registry, mut state, police, neighborhood) = make_fixture();
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, 1_440, 60)],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");
    let stale = validate_revise_patrol_deployment(&state, deployment, vec![window(0, 1_440, 80)])
        .expect("patrol revision should validate");
    validate_patrol_transition(&state, deployment, PatrolDeploymentTransition::Suspend)
        .expect("patrol suspension should validate")
        .commit(&mut state)
        .expect("patrol suspension should commit");

    let error = stale
        .commit(&mut state)
        .expect_err("stale revision must not overwrite lifecycle change");
    assert_eq!(
        error,
        PatrolError::StaleDeployment {
            deployment,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .legal()
            .get_patrol_deployment(deployment)
            .expect("deployment should remain present")
            .status(),
        PatrolDeploymentStatus::Suspended
    );
    validate_invariants(&state);
}

#[test]
fn patrol_deployment_survives_save_round_trip_with_active_index() {
    let (registry, mut state, police, neighborhood) = make_fixture();
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(600, 120, 75)],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");
    let envelope = build_save(&registry, &state).expect("patrol state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("patrol state should restore");

    assert_eq!(
        restored
            .legal()
            .get_patrol_deployment(deployment)
            .expect("restored deployment should exist")
            .version(),
        1
    );
    assert_eq!(
        resolve_patrol_presence(&restored, neighborhood, SimTime::from_minutes(660))
            .map(Rating::value),
        Some(75)
    );
    validate_state(&restored).expect("restored patrol state should validate");
    validate_invariants(&restored);
}
