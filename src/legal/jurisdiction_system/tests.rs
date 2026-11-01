//! Focused tests for jurisdiction assignment and police-response authority resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::legal::JurisdictionDraft;
use crate::world::world_system::{insert_neighborhood, insert_organization};
use crate::world::{
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, Rating,
};
use std::collections::BTreeSet;

fn make_fixture() -> (AppState, NeighborhoodId, OrganizationId, OrganizationId) {
    let registry = build_registry();
    let mut state = AppState::new(0x1A57_1933);
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Jurisdiction Test Ward".to_owned(),
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
    .expect("neighborhood fixture should validate");
    let first = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "First Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("first legal authority should validate");
    let second = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Second Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second legal authority should validate");
    (state, neighborhood, first, second)
}

#[test]
fn case_intake_uses_priority_then_stable_organization_id() {
    let (mut state, neighborhood, first, second) = make_fixture();
    for (organization, priority) in [(first, 70), (second, 85)] {
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(priority)
                    .expect("fixture priority should validate"),
            },
        )
        .expect("jurisdiction fixture should validate")
        .commit(&mut state)
        .expect("jurisdiction fixture should commit");
    }
    assert_eq!(
        resolve_case_intake_authority(&state, neighborhood),
        Some(second)
    );

    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: first,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(85).expect("fixture priority should validate"),
        },
    )
    .expect("priority update should validate")
    .commit(&mut state)
    .expect("priority update should commit");
    assert!(
        first < second,
        "fixture IDs should be allocated in stable order"
    );
    assert_eq!(
        resolve_case_intake_authority(&state, neighborhood),
        Some(first)
    );
    validate_state(&state).expect("jurisdiction state should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn stale_jurisdiction_token_cannot_overwrite_newer_assignment() {
    let (mut state, neighborhood, first, _second) = make_fixture();
    let draft = || JurisdictionDraft {
        organization: first,
        neighborhoods: BTreeSet::from([neighborhood]),
        case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
    };
    let stale = validate_set_jurisdiction(&state, draft())
        .expect("initial jurisdiction token should validate");
    validate_set_jurisdiction(&state, draft())
        .expect("concurrent jurisdiction token should validate")
        .commit(&mut state)
        .expect("newer jurisdiction token should commit");

    let error = stale
        .commit(&mut state)
        .expect_err("stale jurisdiction token must be rejected");
    assert_eq!(
        error,
        JurisdictionError::StaleJurisdiction {
            organization: first,
            expected: None,
            found: Some(1),
        }
    );
    validate_invariants(&state);
}

#[test]
fn criminal_organization_cannot_receive_legal_jurisdiction() {
    let registry = build_registry();
    let (mut state, neighborhood, _first, _second) = make_fixture();
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Not A Precinct".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal organization fixture should validate");
    let error = validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: criminal,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(50).expect("fixture priority should validate"),
        },
    )
    .expect_err("criminal organization must not own legal jurisdiction");
    assert_eq!(error, JurisdictionError::InvalidAuthorityKind(criminal));
    validate_invariants(&state);
}

#[test]
fn identical_jurisdiction_assignment_is_rejected_without_version_churn() {
    let (mut state, neighborhood, first, _second) = make_fixture();
    let draft = || JurisdictionDraft {
        organization: first,
        neighborhoods: BTreeSet::from([neighborhood]),
        case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
    };
    validate_set_jurisdiction(&state, draft())
        .expect("initial jurisdiction should validate")
        .commit(&mut state)
        .expect("initial jurisdiction should commit");
    let before = bincode::serialize(&state).expect("fixture state should serialize");

    let error = validate_set_jurisdiction(&state, draft())
        .expect_err("identical jurisdiction replacement must be rejected");
    assert_eq!(error, JurisdictionError::JurisdictionUnchanged(first));
    assert_eq!(
        bincode::serialize(&state).expect("rejected state should serialize"),
        before,
        "unchanged jurisdiction must not invalidate routing snapshots"
    );
    validate_invariants(&state);
}

#[test]
fn historical_jurisdiction_queries_match_linear_replay_across_same_minute_changes() {
    let (mut state, neighborhood, first, second) = make_fixture();
    for (minute, organization, priority) in [
        (0_u32, first, 50_u8),
        (0, first, 65),
        (3, first, 70),
        (3, second, 90),
        (3, second, 40),
        (3, first, 80),
        (6, second, 95),
        (6, first, 99),
    ] {
        let current = state.now().as_minutes();
        if u64::from(minute) > current {
            state.advance_clock(crate::core::time::SimDuration::from_minutes(
                minute - u32::try_from(current).expect("fixture time fits u32"),
            ));
        }
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(priority).unwrap(),
            },
        )
        .expect("revision must validate")
        .commit(&mut state)
        .expect("revision must commit");
    }
    let check = |state: &AppState| {
        for organization in [first, second] {
            let record = state.legal().get_jurisdiction(organization).unwrap();
            let revisions = record.revisions();
            for minute in 0..=9 {
                let at = crate::core::time::SimTime::from_minutes(minute);
                let expected: Vec<_> = revisions
                    .first()
                    .is_some_and(|revision| revision.changed_at() >= at)
                    .then_some(None)
                    .into_iter()
                    .chain(
                        revisions
                            .iter()
                            .rev()
                            .find(|revision| revision.changed_at() < at)
                            .map(Some),
                    )
                    .chain(
                        revisions
                            .iter()
                            .filter(|revision| revision.changed_at() == at)
                            .map(Some),
                    )
                    .map(|candidate| candidate.map(|revision| revision.version()))
                    .collect();
                let actual: Vec<_> = reachable_jurisdiction_revisions_at(record, at)
                    .map(|candidate| candidate.map(|revision| revision.version()))
                    .collect();
                assert_eq!(actual, expected, "{organization} minute {minute}");
                for revision in revisions {
                    let expected = revision.changed_at() <= at
                        && revisions
                            .iter()
                            .filter(|later| later.version() > revision.version())
                            .all(|later| later.changed_at() >= at);
                    assert_eq!(
                        jurisdiction_revision_is_possible_at(record, revision, at),
                        expected,
                        "{organization} revision {} minute {minute}",
                        revision.version()
                    );
                }
            }
        }
    };
    check(&state);
    validate_state(&state).expect("jurisdiction histories must validate");
    validate_invariants(&state);
    let registry = build_registry();
    let restored = crate::core::persistence::restore_save(
        &registry,
        crate::core::persistence::build_save(&registry, &state).expect("history should save"),
    )
    .expect("history should restore");
    check(&restored);
}
