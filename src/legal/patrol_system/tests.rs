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
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Serialize)]
struct PatrolDeploymentRevisionWire {
    changed_at: SimTime,
    windows: Vec<PatrolWindow>,
    status: PatrolDeploymentStatus,
    version: u32,
}

#[derive(Clone, Serialize)]
struct PatrolDeploymentRecordWire {
    id: PatrolDeploymentId,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    established_at: SimTime,
    revisions: Vec<PatrolDeploymentRevisionWire>,
}

fn patrol_deployment_wire(record: &PatrolDeploymentRecord) -> PatrolDeploymentRecordWire {
    PatrolDeploymentRecordWire {
        id: record.id(),
        organization: record.organization(),
        neighborhood: record.neighborhood(),
        established_at: record.established_at(),
        revisions: record
            .revisions()
            .iter()
            .map(|revision| PatrolDeploymentRevisionWire {
                changed_at: revision.changed_at(),
                windows: revision.windows().to_vec(),
                status: revision.status(),
                version: revision.version(),
            })
            .collect(),
    }
}

fn replace_serialized_patrol_deployment(
    envelope: SaveEnvelope,
    original: &PatrolDeploymentRecord,
    replacement: &PatrolDeploymentRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("patrol deployment should serialize");
    let mirror = patrol_deployment_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("patrol deployment mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement patrol deployment should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized patrol deployment must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout patrol corruption must remain decodable")
}

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

#[test]
fn interval_presence_stays_exact_across_the_full_clock_range() {
    let (_registry, mut state, police, neighborhood) = make_fixture();
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, DAY_MINUTES_U16, 80)],
        },
    )
    .expect("full-day patrol deployment should validate")
    .commit(&mut state)
    .expect("full-day patrol deployment should commit");

    let snapshot = resolve_patrol_presence_interval_snapshot(
        &state,
        neighborhood,
        SimTime::from_minutes(0),
        SimTime::from_minutes(u64::MAX),
    );
    assert_eq!(
        snapshot.presence().map(Rating::value),
        Some(80),
        "interval averaging must not distort presence when u64 accumulation would overflow"
    );
    validate_invariants(&state);
}

#[test]
fn patrol_queries_and_restore_preserve_revision_chronology() {
    let (registry, mut state, police, neighborhood) = make_fixture();
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, DAY_MINUTES_U16, 70)],
        },
    )
    .expect("historical patrol deployment should validate")
    .commit(&mut state)
    .expect("historical patrol deployment should commit");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));
    validate_revise_patrol_deployment(&state, deployment, vec![window(600, 120, 80)])
        .expect("historical patrol revision should validate")
        .commit(&mut state)
        .expect("historical patrol revision should commit");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));
    validate_patrol_transition(&state, deployment, PatrolDeploymentTransition::Suspend)
        .expect("historical patrol suspension should validate")
        .commit(&mut state)
        .expect("historical patrol suspension should commit");

    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(5)).map(Rating::value),
        Some(70)
    );
    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(15)).map(Rating::value),
        Some(0)
    );
    assert_eq!(
        resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(20)),
        None,
        "suspended deployments no longer replace ambient presence with an explicit patrol schedule"
    );
    let interval = resolve_patrol_presence_interval_snapshot(
        &state,
        neighborhood,
        SimTime::ZERO,
        SimTime::from_minutes(30),
    );
    assert_eq!(
        interval.presence().map(Rating::value),
        Some(43),
        "10 minutes at 70, 10 minutes in an explicit gap, then 10 minutes of ambient 60 must average to 43"
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("patrol history should save"),
    )
    .expect("patrol history should restore");
    assert_eq!(
        resolve_patrol_presence(&restored, neighborhood, SimTime::from_minutes(5))
            .map(Rating::value),
        Some(70)
    );
    assert_eq!(
        resolve_patrol_presence(&restored, neighborhood, SimTime::from_minutes(15))
            .map(Rating::value),
        Some(0)
    );
    assert_eq!(
        restored
            .legal()
            .get_patrol_deployment(deployment)
            .expect("restored deployment should persist")
            .version(),
        3
    );
    validate_state(&restored).expect("restored patrol revision history should remain valid");
    validate_invariants(&restored);
}

#[test]
fn restore_rejects_historically_overlapping_patrols_for_one_authority() {
    let (registry, mut state, police, neighborhood) = make_fixture();
    let first = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, DAY_MINUTES_U16, 70)],
        },
    )
    .expect("first patrol should validate")
    .commit(&mut state)
    .expect("first patrol should commit");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));
    validate_patrol_transition(&state, first, PatrolDeploymentTransition::Suspend)
        .expect("first patrol should suspend")
        .commit(&mut state)
        .expect("first patrol suspension should commit");
    let second = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![window(0, DAY_MINUTES_U16, 60)],
        },
    )
    .expect("replacement patrol should validate after suspension")
    .commit(&mut state)
    .expect("replacement patrol should commit");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));

    let original = state
        .legal()
        .get_patrol_deployment(second)
        .expect("replacement patrol should persist");
    let mut corrupted = patrol_deployment_wire(original);
    corrupted.established_at = SimTime::from_minutes(5);
    corrupted.revisions[0].changed_at = SimTime::from_minutes(5);
    let error = restore_save(
        &registry,
        replace_serialized_patrol_deployment(
            build_save(&registry, &state)
                .expect("valid non-overlapping patrol history should save before corruption"),
            original,
            &corrupted,
        ),
    )
    .expect_err("one authority cannot have two historically active deployments in one district");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidPatrolDeployment {
                deployment: invalid
            }
        ) if invalid == second
    ));
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

#[test]
fn patrol_revision_rejects_normalized_unchanged_schedule_without_freshness_churn() {
    let (_registry, mut state, police, neighborhood) = make_fixture();
    let first = window(600, 120, 75);
    let second = window(900, 60, 55);
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![first, second],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");
    let before = bincode::serialize(&state).expect("fixture state should serialize");

    let error = validate_revise_patrol_deployment(&state, deployment, vec![second, first])
        .expect_err("equivalent normalized schedule must be rejected as unchanged");
    assert_eq!(error, PatrolError::ScheduleUnchanged(deployment));
    assert_eq!(
        bincode::serialize(&state).expect("rejected state should serialize"),
        before,
        "unchanged patrol revision must not advance version or last-changed time"
    );
    validate_invariants(&state);
}
