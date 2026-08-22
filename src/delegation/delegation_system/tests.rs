//! Focused tests for mandate lifecycle, revision, revocation, and dependency checks.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::delegation::ResponsibilityFunction;
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

fn make_authority_fixture() -> (crate::Registry, AppState, MandateAuthority) {
    let registry = build_registry();
    let mut state = AppState::new(67);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Authority Test Organization".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("organization fixture should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Authority Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("manager fixture should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Finance,
            )]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate fixture should validate")
    .commit(&mut state)
    .expect("validated mandate should remain current");
    (
        registry,
        state,
        MandateAuthority {
            mandate,
            manager,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Finance),
        },
    )
}

#[test]
fn resolves_authority_with_versioned_dependencies() {
    let (_registry, state, authority) = make_authority_fixture();
    let resolved = resolve_mandate_authority(&state, authority)
        .expect("valid mandate authority should resolve");

    assert_eq!(resolved.authority(), authority);
    assert_eq!(
        resolved.organization(),
        state
            .delegation()
            .get_mandate(authority.mandate)
            .expect("mandate should exist")
            .organization()
    );
    assert_eq!(resolved.mandate_version(), 1);
    assert_eq!(resolved.manager_version(), 1);
    validate_invariants(&state);
}

#[test]
fn authority_rejects_wrong_manager_and_scope() {
    let (_, mut state, authority) = make_authority_fixture();
    let organization = state
        .delegation()
        .get_mandate(authority.mandate)
        .expect("mandate should exist")
        .organization();
    let other_manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Other Authority Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second manager fixture should validate");

    let wrong_manager = MandateAuthority {
        manager: other_manager,
        ..authority
    };
    assert_eq!(
        resolve_mandate_authority(&state, wrong_manager)
            .expect_err("another manager must not exercise the mandate"),
        DelegationError::AuthorityManagerMismatch {
            mandate: authority.mandate,
            manager: other_manager,
            expected: authority.manager,
        }
    );

    let wrong_scope = MandateAuthority {
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Operations),
        ..authority
    };
    assert_eq!(
        resolve_mandate_authority(&state, wrong_scope)
            .expect_err("authority must remain inside the mandate scope"),
        DelegationError::ScopeOutsideMandate {
            mandate: authority.mandate,
            scope: wrong_scope.scope,
        }
    );
    validate_invariants(&state);
}

#[test]
fn authority_snapshot_rejects_later_mandate_revision() {
    let (_, mut state, authority) = make_authority_fixture();
    let snapshot = resolve_mandate_authority(&state, authority)
        .expect("valid authority should resolve before revision");
    validate_revise_mandate(
        &state,
        authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([
                ResponsibilityScope::Function(ResponsibilityFunction::Finance),
                ResponsibilityScope::Function(ResponsibilityFunction::Operations),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("mandate revision should validate")
    .commit(&mut state)
    .expect("mandate revision should commit");

    assert_eq!(
        ensure_mandate_authority_current(&state, snapshot)
            .expect_err("authority snapshot must become stale after mandate revision"),
        DelegationError::StaleMandate {
            mandate: authority.mandate,
            expected: 1,
            found: 2,
        }
    );
    validate_invariants(&state);
}
