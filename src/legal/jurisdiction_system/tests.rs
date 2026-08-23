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
